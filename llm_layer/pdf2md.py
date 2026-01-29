#!/usr/bin/env python3
"""
Convert a directory of PDFs to Markdown.

Default: uses pdfminer.six text extraction.
Optional: --ocr falls back to Tesseract OCR via pdf2image for image-only PDFs.

Usage:
    python pdfs_to_md.py INPUT_DIR OUTPUT_DIR [--ocr] [--lang eng]
                        [--preserve-layout] [--min-chars 200] [--overwrite]
"""

import argparse
import sys
import shutil
from pathlib import Path
from typing import Optional

# --- Text extraction (pdfminer) ---
from pdfminer.high_level import extract_text
from pdfminer.layout import LAParams

# --- Optional OCR fallback ---
try:
    from pdf2image import convert_from_path
    import pytesseract
    OCR_AVAILABLE = True
except Exception:
    OCR_AVAILABLE = False


def extract_text_pdfminer(pdf_path: Path, preserve_layout: bool = False) -> str:
    """
    Extract text with pdfminer.six. Optionally preserve layout for better tables/spacing.
    """
    laparams = None
    if preserve_layout:
        laparams = LAParams(
            line_margin=0.1,  # tighter line spacing
            char_margin=2.0,
            word_margin=0.1,
            detect_vertical=False,
            all_texts=True
        )
    try:
        text = extract_text(str(pdf_path), laparams=laparams) or ""
    except Exception as e:
        print(f"[WARN] pdfminer failed on {pdf_path.name}: {e}", file=sys.stderr)
        text = ""
    return text


def extract_text_ocr(pdf_path: Path, lang: str = "eng", dpi: int = 300) -> str:
    """
    OCR each page by rendering to images (via poppler) and reading with Tesseract.
    Requires: poppler-utils (pdftoppm/pdftocairo) and tesseract-ocr installed.
    """
    if not OCR_AVAILABLE:
        return ""

    try:
        images = convert_from_path(str(pdf_path), dpi=dpi)
    except Exception as e:
        print(f"[WARN] pdf2image failed on {pdf_path.name}: {e}", file=sys.stderr)
        return ""

    ocr_pages = []
    for i, img in enumerate(images, start=1):
        try:
            page_text = pytesseract.image_to_string(img, lang=lang)
        except Exception as e:
            print(f"[WARN] Tesseract failed on {pdf_path.name} page {i}: {e}", file=sys.stderr)
            page_text = ""
        # Page break marker for readability in Markdown
        ocr_pages.append(page_text.strip())

    # Separate pages with an em-dash rule so Markdown viewers show a divider
    return "\n\n---\n\n".join(ocr_pages).strip()


def to_markdown(title: str, body: str) -> str:
    """Basic Markdown wrapper with a title."""
    title = title.strip()
    body = body.replace("\r\n", "\n").strip()
    return f"# {title}\n\n{body}\n"


def convert_one_pdf(
    pdf_path: Path,
    out_dir: Path,
    preserve_layout: bool,
    use_ocr: bool,
    ocr_lang: str,
    min_chars: int,
    overwrite: bool
) -> Optional[Path]:
    """
    Convert a single PDF to Markdown.
    Returns the output .md path on success, or None on failure/skip.
    """
    rel_stem = pdf_path.stem
    out_path = out_dir / f"{rel_stem}.md"

    if out_path.exists() and not overwrite:
        print(f"[SKIP] {out_path} (exists)")
        return None

    # 1) Try text layer first
    text = extract_text_pdfminer(pdf_path, preserve_layout=preserve_layout).strip()

    # 2) If text looks too short or empty, optionally OCR
    if use_ocr and len(text) < min_chars:
        ocr_text = extract_text_ocr(pdf_path, lang=ocr_lang)
        if len(ocr_text) > len(text):
            text = ocr_text

    # 3) If still empty, give up gracefully
    if len(text.strip()) == 0:
        print(f"[WARN] No text extracted from {pdf_path.name}. Consider --ocr and installing poppler & tesseract.")
        return None

    md = to_markdown(rel_stem, text)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(md, encoding="utf-8")
    print(f"[OK] {pdf_path.name} -> {out_path.relative_to(out_dir.parent) if out_dir.parent in out_path.parents else out_path}")
    return out_path


def main():
    parser = argparse.ArgumentParser(description="Convert PDFs in a directory to Markdown.")
    parser.add_argument("input_dir", type=Path, help="Directory containing PDFs (searched recursively).")
    parser.add_argument("output_dir", type=Path, help="Directory to write .md files.")
    parser.add_argument("--ocr", action="store_true", help="Enable OCR fallback for image-only PDFs.")
    parser.add_argument("--lang", default="eng", help="Tesseract language code (default: eng).")
    parser.add_argument("--preserve-layout", action="store_true",
                        help="Ask pdfminer to preserve layout (helps tables but may add extra spaces).")
    parser.add_argument("--min-chars", type=int, default=200,
                        help="If pdfminer returns fewer than this many chars, try OCR (when --ocr is set).")
    parser.add_argument("--overwrite", action="store_true", help="Overwrite existing .md files.")
    args = parser.parse_args()

    if args.ocr:
        # Check for external tools early to give informative warnings
        if not OCR_AVAILABLE:
            print("[ERROR] OCR requested but 'pdf2image'/'pytesseract' not available. Install python deps and system tools.", file=sys.stderr)
            sys.exit(2)
        # Poppler presence hint
        if shutil.which("pdftoppm") is None and shutil.which("pdftocairo") is None:
            print("[WARN] Poppler not found. Install 'poppler-utils' (Linux) or 'brew install poppler' (macOS).", file=sys.stderr)
        if shutil.which("tesseract") is None:
            print("[WARN] Tesseract not found. Install 'tesseract-ocr'.", file=sys.stderr)

    in_dir = args.input_dir
    out_dir = args.output_dir

    if not in_dir.exists() or not in_dir.is_dir():
        print(f"[ERROR] Input directory not found: {in_dir}", file=sys.stderr)
        sys.exit(1)

    out_dir.mkdir(parents=True, exist_ok=True)

    pdfs = sorted([p for p in in_dir.rglob("*.pdf") if p.is_file()])
    if not pdfs:
        print("[INFO] No PDFs found.")
        return

    print(f"[INFO] Found {len(pdfs)} PDF(s). Converting...")
    successes = 0
    for pdf in pdfs:
        try:
            if convert_one_pdf(
                pdf,
                out_dir,
                preserve_layout=args.preserve_layout,
                use_ocr=args.ocr,
                ocr_lang=args.lang,
                min_chars=args.min_chars,
                overwrite=args.overwrite
            ):
                successes += 1
        except KeyboardInterrupt:
            print("\n[ABORTED] Interrupted by user.")
            sys.exit(130)
        except Exception as e:
            print(f"[ERROR] Failed on {pdf.name}: {e}", file=sys.stderr)

    print(f"[DONE] Converted {successes}/{len(pdfs)} PDF(s) to Markdown.")


if __name__ == "__main__":
    main()
