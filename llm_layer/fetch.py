#!/usr/bin/env python3
"""
oa_pubmed_to_pdf.py

Open-access only pipeline:

1) PubMed (Metapub) search with PMC subset:
     (<query>) AND "pubmed pmc"[sb]
   -> PMIDs (relevance order) -> Article objects

2) For each article:
   - DOI recorded if present
   - If DOI is BioRxiv/medRxiv (10.1101/...), use bio2csv to resolve the preprint page
     and extract a direct PDF link, then download it.
   - Else, use PMCID to download the PDF from PMC:
       https://www.ncbi.nlm.nih.gov/pmc/articles/<PMCID>/pdf
     (fallback: parse article page for a .pdf link)

3) Verify '%PDF-' magic header; only count as saved if it's a real PDF.

Outputs:
- PDFs in --outdir
- manifest.csv with: pmid, pmcid, doi, title, first_author, year, journal, filename, source
"""

import argparse
import csv
import re
import sys
import time
from pathlib import Path
from urllib.parse import urljoin

import requests
from metapub import PubMedFetcher, FindIt
from bio2csv import fetch_paper_details

# ---------------- Config ----------------
REQUEST_SLEEP = 0.0
UA = "OA-PubMed-to-PDF/1.0 (+https://example.org)"
HEADERS = {"User-Agent": UA, "Accept": "*/*"}

BIOXRIV_HOST = "https://www.biorxiv.org"
MEDXRIV_HOST = "https://www.medrxiv.org"

# --------------- Helpers ----------------

def sanitize_filename(name: str, maxlen: int = 180) -> str:
    name = re.sub(r"[\\/:*?\"<>|\n\r\t]+", " ", name)
    name = re.sub(r"\s+", " ", name).strip()
    return (name[:maxlen].rstrip() or "untitled")

def is_pdf_file(path: Path) -> bool:
    try:
        if not path.exists() or path.stat().st_size < 5:
            return False
        with open(path, "rb") as f:
            return f.read(5) == b"%PDF-"
    except Exception:
        return False

def head_is_pdf_url(url: str, timeout: int = 20) -> bool:
    try:
        r = requests.head(url, allow_redirects=True, timeout=timeout, headers=HEADERS)
        ctype = (r.headers.get("Content-Type") or "").lower()
        if "pdf" in ctype:
            return True
        # Some servers don't support HEAD; check with a small GET
        r = requests.get(url, stream=True, allow_redirects=True, timeout=timeout, headers=HEADERS)
        ctype = (r.headers.get("Content-Type") or "").lower()
        return "pdf" in ctype
    except requests.RequestException:
        return False

def download_pdf_url(url: str, outpath: Path, timeout: int = 90) -> bool:
    try:
        with requests.get(url, stream=True, allow_redirects=True, timeout=timeout, headers=HEADERS) as r:
            r.raise_for_status()
            with open(outpath, "wb") as f:
                for chunk in r.iter_content(1024 * 64):
                    if chunk:
                        f.write(chunk)
        return is_pdf_file(outpath)
    except Exception:
        outpath.unlink(missing_ok=True)
        return False

def first_author_str(article) -> str:
    try:
        if not article or not getattr(article, "authors", None):
            return "NA"
        a0 = article.authors[0]
        if isinstance(a0, str):
            return (a0.split(",")[0] if "," in a0 else a0.split()[0]).strip() or "NA"
        if isinstance(a0, dict):
            for k in ("last_fm", "lastname", "name"):
                if a0.get(k):
                    return str(a0[k]).split()[0]
        for attr in ("last_fm", "lastname", "name"):
            if hasattr(a0, attr):
                v = getattr(a0, attr)
                if v:
                    return str(v).split()[0]
    except Exception:
        pass
    return "NA"

def doi_is_preprint(doi: str) -> bool:
    # BioRxiv/medRxiv use Crossref prefix 10.1101
    return doi and doi.lower().startswith("10.1101/")

def biorxiv_pdf_from_page(html: str, base: str) -> list[str]:
    # Typical BioRxiv/medRxiv pattern: /content/10.1101/xxxxxvN.full.pdf
    hrefs = re.findall(r'href="([^"]+\.full\.pdf)"', html, flags=re.IGNORECASE)
    urls = []
    for h in hrefs:
        urls.append(h if h.startswith("http") else urljoin(base, h))
    # de-dup, keep order
    seen, out = set(), []
    for u in urls:
        if u not in seen:
            seen.add(u)
            out.append(u)
    return out

def resolve_preprint_pdf_url(doi: str) -> str | None:
    """
    Resolve BioRxiv/medRxiv direct PDF using bio2csv to fetch article page HTML.
    Try BioRxiv first, then medRxiv; fallback to canonical guess.
    """
    import requests as _rq
    sess = _rq.Session()

    # Try BioRxiv
    try:
        bx_url = f"{BIOXRIV_HOST}/content/{doi}"
        _, html = fetch_paper_details(bx_url, sess)  # returns (abstract, full_html)
        if html:
            for u in biorxiv_pdf_from_page(html, BIOXRIV_HOST):
                if head_is_pdf_url(u):
                    return u
    except Exception:
        pass

    # Try medRxiv
    try:
        mx_url = f"{MEDXRIV_HOST}/content/{doi}"
        _, html = fetch_paper_details(mx_url, sess)
        if html:
            for u in biorxiv_pdf_from_page(html, MEDXRIV_HOST):
                if head_is_pdf_url(u):
                    return u
    except Exception:
        pass

    # Fallback guess
    for host in (BIOXRIV_HOST, MEDXRIV_HOST):
        guess = f"{host}/content/{doi}.full.pdf"
        if head_is_pdf_url(guess):
            return guess
    return None

def pmcid_pdf_url(pmcid: str) -> str:
    return f"https://www.ncbi.nlm.nih.gov/pmc/articles/{pmcid}/pdf"

def pmcid_first_pdf_from_page(pmcid: str) -> str | None:
    # Fallback: parse article page for any .pdf link
    page = f"https://www.ncbi.nlm.nih.gov/pmc/articles/{pmcid}/"
    try:
        r = requests.get(page, headers=HEADERS, timeout=30)
        r.raise_for_status()
        matches = re.findall(r'href="([^"]+\.pdf)"', r.text, flags=re.IGNORECASE)
        for href in matches:
            url = href if href.startswith("http") else urljoin(page, href)
            if head_is_pdf_url(url):
                return url
    except Exception:
        return None
    return None

# --------------- Main -------------------

def main():
    ap = argparse.ArgumentParser(description="Search PubMed OA (PMC subset), save PDFs (bio2csv for preprints, PMC for others).")
    ap.add_argument("--query", required=True, help="PubMed search string (we will add the PMC subset filter)")
    ap.add_argument("--top", type=int, default=30, help="Number of PDFs to save")
    ap.add_argument("--search-pool", type=int, default=200, help="How many PubMed results to scan")
    ap.add_argument("--outdir", default="oa_pubmed_pdfs", help="Output directory")
    ap.add_argument("--manifest", default=None, help="CSV manifest path (default: <outdir>/manifest.csv)")
    args = ap.parse_args()

    outdir = Path(args.outdir)
    outdir.mkdir(parents=True, exist_ok=True)
    manifest = Path(args.manifest) if args.manifest else (outdir / "manifest.csv")

    # Enforce OA by using the PMC subset
    pmc_query = f'({args.query}) AND "pubmed pmc"[sb]'
    print(f"Searching PubMed (PMC subset) for: {pmc_query!r}")

    fetch = PubMedFetcher()  # honors NCBI_API_KEY
    try:
        pmids = fetch.pmids_for_query(query=pmc_query, retmax=args.search_pool)
    except Exception as e:
        print(f"Failed to query PubMed: {e}")
        sys.exit(1)

    if not pmids:
        print("No PMC (OA) results found.")
        sys.exit(0)

    print(f"Got {len(pmids)} PMIDs (OA). Fetching articles and downloading PDFs...")
    saved, rows = 0, []
    for idx, pmid in enumerate(pmids, start=1):
        try:
            art = fetch.article_by_pmid(pmid)
        except Exception as e:
            print(f"[{idx:03d}] PMID {pmid}: error fetching metadata: {e}")
            time.sleep(REQUEST_SLEEP)
            continue

        title  = (getattr(art, "title", "") or f"PMID{pmid}").strip()
        year   = str(getattr(art, "year", "") or "").strip()
        journal = (getattr(art, "journal", "") or "").strip()
        first  = first_author_str(art)
        doi    = getattr(art, "doi", None)
        pmcid  = getattr(art, "pmcid", None)

        # Build filename
        base = sanitize_filename(f"{first or 'NA'}_{year or 'NA'}_{title}")
        outfile = outdir / f"{base}.pdf"
        k = 1
        while outfile.exists():
            outfile = outdir / f"{base}_{k}.pdf"
            k += 1

        source = None
        ok = False

        # Prefer bioRxiv/medRxiv path if it's clearly a preprint DOI
        if doi_is_preprint(doi or ""):
            print(f"[{idx:03d}] PMID {pmid}: preprint DOI {doi} → resolving via bio2csv…")
            pdf_url = resolve_preprint_pdf_url(doi)
            if pdf_url and download_pdf_url(pdf_url, outfile):
                source, ok = "bio2csv-preprint", True

        # Otherwise fetch from PMC (we restricted to PMC subset -> PMCID should be present)
        if not ok:
            # Try FindIt first (often gives the cleanest PDF URL)
            try:
                src = FindIt(pmid)
                pdf = getattr(src, "pmcid_pdf_url", None) or getattr(src, "url", None)
            except Exception:
                pdf = None

            if not pdf and pmcid:
                # Direct canonical PMC PDF URL
                pdf = pmcid_pdf_url(pmcid)

            if not pdf and pmcid:
                # Parse page for a .pdf link as last resort
                pdf = pmcid_first_pdf_from_page(pmcid)

            if pdf and download_pdf_url(pdf, outfile):
                source, ok = "pmc", True

        if ok:
            rows.append({
                "pmid": pmid,
                "pmcid": pmcid or "",
                "doi": doi or "",
                "title": title,
                "first_author": first,
                "year": year,
                "journal": journal,
                "filename": outfile.name,
                "source": source,
            })
            saved += 1
            print(f"      Saved ({source}): {outfile.name}")
        else:
            print(f"      Skipped: could not obtain a valid PDF (PMID {pmid})")

        time.sleep(REQUEST_SLEEP)
        if saved >= args.top:
            break

    # Write manifest
    if rows:
        with open(manifest, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=[
                "pmid","pmcid","doi","title","first_author","year","journal","filename","source"
            ])
            w.writeheader()
            w.writerows(rows)

    print(f"\nDone. Saved PDFs: {saved}. Manifest: {manifest}")

if __name__ == "__main__":
    main()
