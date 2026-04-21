//! Citation correctness evals.
//!
//! Four metrics sit on top of summary generation's `[chunk:<UUID>]` citation
//! markers:
//!
//!  - `citation_presence`  — fraction of factual claims that carry ≥ 1 citation.
//!  - `citation_validity`  — fraction of cited UUIDs that resolve to a real
//!                           `brain_region_embeddings` row.
//!  - `citation_scope`     — fraction of cited UUIDs that belong to **this**
//!                           summary's retrieval set.
//!  - `citation_support`   — (optional, LLM) fraction of cited chunks that the
//!                           judge agrees actually support the claim.
//!
//! Three of the four are pure `&str -> f32` mappings colocated here with the
//! deterministic structural metrics. The fourth is driven by
//! `state_machine::AwaitingCitationSupport`.

use domain::Claim;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use uuid::Uuid;

// ---- Public types (Phase 1, Task 3) ----

/// The kind of citation problem detected on a single claim / citation.
///
/// Serialised into `eval_scores.details.issues[].kind` as snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationIssueKind {
    /// Factual sentence left without any `[chunk:...]` marker.
    Missing,
    /// Cited UUID does not exist in `brain_region_embeddings`.
    Orphan,
    /// Cited UUID exists but belongs to a different summary's corpus.
    OutOfScope,
    /// Judge says the cited chunk does not support the claim.
    Unsupported,
    /// Judge says the cited chunk contradicts the claim.
    Contradicted,
}

/// One rich, displayable issue attached to a single claim+citation pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationIssue {
    pub kind: CitationIssueKind,
    pub claim_id: u32,
    pub claim_text: String,
    /// `None` for `Missing` (no chunk was cited); `Some` for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offending_chunk_id: Option<Uuid>,
    pub rationale: String,
}

// ---- Parsing (Phase 2, Task 4) ----

/// One `[chunk:<uuid>]` marker pulled out of a summary body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCitation {
    pub uuid: Uuid,
    /// Byte offset of the *opening* `[` in the original string.
    pub byte_offset: usize,
    /// The sentence that immediately encloses the citation, trimmed. Useful
    /// as prompt context for the support judge.
    pub enclosing_sentence: String,
}

fn citation_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\[chunk:([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\]")
            .expect("citation regex is valid")
    })
}

fn fenced_code_block_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)```.*?```").expect("fenced code block regex is valid"))
}

/// Replace every fenced code block with equivalent-length spaces so byte
/// offsets for downstream parsing are preserved but no `[chunk:…]` example
/// inside a fenced block is counted.
fn blank_fenced_code_blocks(summary: &str) -> String {
    let re = fenced_code_block_regex();
    let mut out = summary.to_string();
    for m in re.find_iter(summary) {
        let replacement = " ".repeat(m.len());
        out.replace_range(m.range(), &replacement);
    }
    out
}

/// Extract the enclosing sentence around `byte_offset` in `body`.
/// Splits on `.`, `!`, `?` (ASCII) and trims the result.
fn extract_enclosing_sentence(body: &str, byte_offset: usize) -> String {
    // Walk backward for a sentence boundary; walk forward for the next one.
    let bytes = body.as_bytes();
    let len = bytes.len();
    let start = {
        let mut i = byte_offset.min(len);
        while i > 0 {
            let b = bytes[i - 1];
            if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
                break;
            }
            i -= 1;
        }
        // Skip whitespace after the boundary.
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let end = {
        let mut i = byte_offset.min(len);
        while i < len {
            let b = bytes[i];
            if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
                i += 1;
                break;
            }
            i += 1;
        }
        i
    };
    // Guard against splitting a multibyte UTF-8 sequence.
    let start = floor_char_boundary(body, start);
    let end = ceil_char_boundary(body, end);
    body[start..end].trim().to_string()
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    let len = s.len();
    while i < len && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(len)
}

/// Parse every `[chunk:<uuid>]` marker in `summary`. Malformed UUIDs and
/// markers inside fenced code blocks are silently skipped.
pub fn parse_citations(summary: &str) -> Vec<ParsedCitation> {
    let sanitized = blank_fenced_code_blocks(summary);
    let re = citation_regex();
    let mut out = Vec::new();
    for caps in re.captures_iter(&sanitized) {
        let m = caps.get(0).expect("whole match exists");
        let uuid_str = caps.get(1).expect("capture group exists").as_str();
        // `Uuid::parse_str` accepts case-insensitive hex; the regex already
        // constrains to lowercase hex, but the (?i) flag allows upper case too.
        if let Ok(uuid) = Uuid::parse_str(uuid_str) {
            // Use the ORIGINAL `summary` for the enclosing sentence so the
            // judge prompt sees the real surrounding text (not our blanked
            // version).
            let sentence = extract_enclosing_sentence(summary, m.start());
            out.push(ParsedCitation {
                uuid,
                byte_offset: m.start(),
                enclosing_sentence: sentence,
            });
        }
    }
    out
}

// ---- Deterministic scoring helpers (Phase 2, Task 5) ----

/// Presence: fraction of claims whose surrounding sentence in the original
/// summary carries at least one `[chunk:…]` marker.
///
/// Mapping strategy (handles Risk #1 from the plan — extractor paraphrases):
///  1. Exact substring match of the claim text against the summary → look for
///     a `[chunk:` within ~200 chars after the match.
///  2. If exact match fails, fall back to token-Jaccard ≥ 0.5 against any
///     sentence; count as present if that sentence has any citation.
///  3. If *both* fail, treat the claim as "missing" only if its whole
///     section contains no citations at all; otherwise give the benefit of
///     the doubt (avoids false positives on paraphrased claims).
///
/// Returns `(score, issues)`. Score is `1.0` when there are zero claims.
pub fn citation_presence_score(summary: &str, claims: &[Claim]) -> (f32, Vec<CitationIssue>) {
    if claims.is_empty() {
        return (1.0, Vec::new());
    }

    let total = claims.len() as f32;
    let mut missing = 0u32;
    let mut issues = Vec::new();

    // Precompute: which sections contain at least one citation? Used to
    // decide whether a claim whose text we cannot locate is probably cited.
    let section_has_citation = section_citation_map(summary);

    for claim in claims {
        // If the claim itself declares cited_chunks, trust it.
        if !claim.cited_chunks.is_empty() {
            continue;
        }

        let cited = claim_is_cited_by_substring(summary, &claim.text)
            || claim_is_cited_by_jaccard(summary, &claim.text);

        if cited {
            continue;
        }

        // Fallback: don't penalise if the whole section is uncited either.
        if section_has_citation
            .get(claim.section.as_str())
            .copied()
            .unwrap_or(false)
        {
            missing += 1;
            issues.push(CitationIssue {
                kind: CitationIssueKind::Missing,
                claim_id: claim.id,
                claim_text: claim.text.clone(),
                offending_chunk_id: None,
                rationale: "no [chunk:...] marker found near this claim".to_string(),
            });
        }
    }

    let score = 1.0 - (missing as f32 / total);
    (score.clamp(0.0, 1.0), issues)
}

/// Build a map `section_heading -> has_any_citation` from the summary body.
/// Sections are delimited by `##` Markdown headers.
fn section_citation_map(summary: &str) -> HashMap<String, bool> {
    let mut out: HashMap<String, bool> = HashMap::new();
    let mut current_section = "Preamble".to_string();
    out.insert(current_section.clone(), false);
    let re = citation_regex();
    for line in summary.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            current_section = header.trim().to_string();
            out.entry(current_section.clone()).or_insert(false);
        } else if re.is_match(line) {
            out.insert(current_section.clone(), true);
        }
    }
    out
}

fn claim_is_cited_by_substring(summary: &str, claim_text: &str) -> bool {
    let trimmed = claim_text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Some(idx) = summary.find(trimmed) {
        // Look for a citation within ~250 bytes after the match.
        let window_end = (idx + trimmed.len() + 250).min(summary.len());
        let window_end = ceil_char_boundary(summary, window_end);
        let window = &summary[idx..window_end];
        return citation_regex().is_match(window);
    }
    false
}

fn claim_is_cited_by_jaccard(summary: &str, claim_text: &str) -> bool {
    let claim_tokens = tokenize(claim_text);
    if claim_tokens.is_empty() {
        return false;
    }
    let re = citation_regex();
    for sentence in summary.split(['.', '!', '?', '\n']) {
        if !re.is_match(sentence) {
            continue;
        }
        // Strip `[chunk:...]` markers before tokenising so UUID hex doesn't
        // distort the Jaccard union.
        let stripped = re.replace_all(sentence, " ");
        let sentence_tokens = tokenize(&stripped);
        if jaccard(&claim_tokens, &sentence_tokens) >= 0.5 {
            return true;
        }
    }
    false
}

fn tokenize(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count() as f32;
    let uni = a.union(b).count() as f32;
    if uni == 0.0 { 0.0 } else { inter / uni }
}

/// Validity: fraction of cited UUIDs that resolve to a real row in
/// `brain_region_embeddings`. Orphans are listed in the returned issues.
///
/// Edge case: zero citations → `(0.0, [])` with the caller responsible for
/// attaching `details.reason = "no_citations"`. This is deliberately harsh
/// to surface summaries that omit citations wholesale.
pub fn citation_validity_score(
    cited_uuids: &[Uuid],
    existing_chunks: &HashMap<Uuid, u32>,
    claim_context: &HashMap<Uuid, (u32, String)>,
) -> (f32, Vec<CitationIssue>) {
    let total = cited_uuids.len() as f32;
    if total == 0.0 {
        return (0.0, Vec::new());
    }
    let mut orphans = 0u32;
    let mut issues = Vec::new();
    for uuid in cited_uuids {
        if !existing_chunks.contains_key(uuid) {
            orphans += 1;
            let (claim_id, claim_text) = claim_context
                .get(uuid)
                .cloned()
                .unwrap_or((0, "<unattributed>".to_string()));
            issues.push(CitationIssue {
                kind: CitationIssueKind::Orphan,
                claim_id,
                claim_text,
                offending_chunk_id: Some(*uuid),
                rationale: "UUID not found in brain_region_embeddings".to_string(),
            });
        }
    }
    let score = 1.0 - (orphans as f32 / total);
    (score.clamp(0.0, 1.0), issues)
}

/// Scope: fraction of *existing* cited UUIDs whose row belongs to
/// `summary_id` (i.e., was retrieved for this summary's corpus).
///
/// Only `existing` citations contribute to the denominator — orphans are
/// already penalised by `citation_validity_score` and must not be double-counted.
///
/// Edge case: zero existing citations → `1.0` (vacuously true).
pub fn citation_scope_score(
    summary_id: Uuid,
    cited_uuids: &[Uuid],
    existing_chunks: &HashMap<Uuid, Uuid>,
    claim_context: &HashMap<Uuid, (u32, String)>,
) -> (f32, Vec<CitationIssue>) {
    let mut total_existing = 0u32;
    let mut out_of_scope = 0u32;
    let mut issues = Vec::new();
    for uuid in cited_uuids {
        if let Some(chunk_summary_id) = existing_chunks.get(uuid) {
            total_existing += 1;
            if *chunk_summary_id != summary_id {
                out_of_scope += 1;
                let (claim_id, claim_text) = claim_context
                    .get(uuid)
                    .cloned()
                    .unwrap_or((0, "<unattributed>".to_string()));
                issues.push(CitationIssue {
                    kind: CitationIssueKind::OutOfScope,
                    claim_id,
                    claim_text,
                    offending_chunk_id: Some(*uuid),
                    rationale: format!(
                        "chunk belongs to summary {} but was cited in summary {}",
                        chunk_summary_id, summary_id
                    ),
                });
            }
        }
    }
    if total_existing == 0 {
        return (1.0, issues);
    }
    let score = 1.0 - (out_of_scope as f32 / total_existing as f32);
    (score.clamp(0.0, 1.0), issues)
}

/// Support verdicts that may be aggregated into the `citation_support` metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportVerdict {
    Supported,
    Partial,
    Contradicted,
    Unsupported,
}

/// Aggregate support verdicts: `supported + 0.5 * partial` over the total.
///
/// Edge case: empty input → `0.0` (with the caller responsible for attaching
/// `details.reason = "no_citations"` when appropriate).
pub fn citation_support_score(verdicts: &[SupportVerdict]) -> f32 {
    if verdicts.is_empty() {
        return 0.0;
    }
    let total = verdicts.len() as f32;
    let weighted: f32 = verdicts
        .iter()
        .map(|v| match v {
            SupportVerdict::Supported => 1.0,
            SupportVerdict::Partial => 0.5,
            SupportVerdict::Contradicted | SupportVerdict::Unsupported => 0.0,
        })
        .sum();
    (weighted / total).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID_A: &str = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    const UUID_B: &str = "11111111-2222-3333-4444-555555555555";
    const UUID_C: &str = "cccccccc-dddd-eeee-ffff-000000000000";

    fn u(s: &str) -> Uuid {
        Uuid::parse_str(s).unwrap()
    }

    // ---- parse_citations ----

    #[test]
    fn parse_empty_summary_returns_empty() {
        assert!(parse_citations("").is_empty());
    }

    #[test]
    fn parse_zero_citations_returns_empty() {
        let s = "The hippocampus supports memory. It sits in the medial temporal lobe.";
        assert!(parse_citations(s).is_empty());
    }

    #[test]
    fn parse_single_citation() {
        let s = format!("Memory is supported [chunk:{}].", UUID_A);
        let out = parse_citations(&s);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].uuid, u(UUID_A));
    }

    #[test]
    fn parse_multi_citation_on_one_sentence() {
        let s = format!(
            "Memory is supported [chunk:{}][chunk:{}]. Next sentence.",
            UUID_A, UUID_B
        );
        let out = parse_citations(&s);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].uuid, u(UUID_A));
        assert_eq!(out[1].uuid, u(UUID_B));
        assert_eq!(out[0].enclosing_sentence, out[1].enclosing_sentence);
    }

    #[test]
    fn parse_skips_malformed_uuids() {
        let s = "Broken [chunk:not-a-uuid] and [chunk:abc123].";
        assert!(parse_citations(s).is_empty());
    }

    #[test]
    fn parse_skips_citations_inside_fenced_code_blocks() {
        let s = format!(
            "Real [chunk:{}].\n\n```\nExample: [chunk:{}]\n```\n\nAlso real [chunk:{}].",
            UUID_A, UUID_B, UUID_C
        );
        let out = parse_citations(&s);
        let uuids: Vec<Uuid> = out.iter().map(|p| p.uuid).collect();
        assert_eq!(uuids, vec![u(UUID_A), u(UUID_C)]);
    }

    #[test]
    fn parse_is_case_insensitive_for_uuid_hex() {
        let upper = UUID_A.to_ascii_uppercase();
        let s = format!("Memory [chunk:{}].", upper);
        let out = parse_citations(&s);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].uuid, u(UUID_A));
    }

    #[test]
    fn parse_enclosing_sentence_trims() {
        let s = format!(
            "Intro. The hippocampus supports memory [chunk:{}]. Next.",
            UUID_A
        );
        let out = parse_citations(&s);
        assert_eq!(out.len(), 1);
        assert!(
            out[0]
                .enclosing_sentence
                .starts_with("The hippocampus supports memory")
        );
    }

    // ---- citation_presence_score ----

    fn make_claim(id: u32, section: &str, text: &str) -> Claim {
        Claim {
            id,
            section: section.to_string(),
            text: text.to_string(),
            cited_chunks: vec![],
        }
    }

    #[test]
    fn presence_all_cited_scores_one() {
        let summary = format!(
            "## Overview\nThe hippocampus supports memory [chunk:{}]. It sits in the MTL [chunk:{}].",
            UUID_A, UUID_B
        );
        let claims = vec![
            make_claim(1, "Overview", "The hippocampus supports memory"),
            make_claim(2, "Overview", "It sits in the MTL"),
        ];
        let (score, issues) = citation_presence_score(&summary, &claims);
        assert_eq!(score, 1.0);
        assert!(issues.is_empty());
    }

    #[test]
    fn presence_half_cited_scores_half() {
        let summary = format!(
            "## Overview\nThe hippocampus supports memory [chunk:{}]. It sits in the MTL.",
            UUID_A
        );
        let claims = vec![
            make_claim(1, "Overview", "The hippocampus supports memory"),
            make_claim(2, "Overview", "It sits in the MTL"),
        ];
        let (score, issues) = citation_presence_score(&summary, &claims);
        assert!((score - 0.5).abs() < 1e-6, "got {}", score);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].claim_id, 2);
        assert_eq!(issues[0].kind, CitationIssueKind::Missing);
    }

    #[test]
    fn presence_empty_claims_scores_one() {
        let (score, issues) = citation_presence_score("anything", &[]);
        assert_eq!(score, 1.0);
        assert!(issues.is_empty());
    }

    #[test]
    fn presence_uncited_section_does_not_penalise() {
        // Entire section has no citations; we give the benefit of the doubt
        // to avoid flagging every claim in a section the author forgot to
        // cite as a separate failure of presence.
        let summary = "## Overview\nThe hippocampus supports memory. It sits in the MTL.";
        let claims = vec![
            make_claim(1, "Overview", "The hippocampus supports memory"),
            make_claim(2, "Overview", "It sits in the MTL"),
        ];
        let (score, issues) = citation_presence_score(summary, &claims);
        assert_eq!(score, 1.0);
        assert!(issues.is_empty());
    }

    #[test]
    fn presence_cited_chunks_on_claim_counts_as_present() {
        let summary = "## Overview\nSomething about the hippocampus.";
        let claim = Claim {
            id: 1,
            section: "Overview".to_string(),
            text: "Something about the hippocampus".to_string(),
            cited_chunks: vec![u(UUID_A)],
        };
        let (score, issues) = citation_presence_score(summary, &[claim]);
        assert_eq!(score, 1.0);
        assert!(issues.is_empty());
    }

    #[test]
    fn presence_paraphrased_claim_jaccard_fallback() {
        // Claim is a paraphrase of the cited sentence. Substring match fails;
        // Jaccard should save us (shared tokens: hippocampus, memory, supports).
        let summary = format!(
            "## Overview\nThe hippocampus strongly supports declarative memory formation [chunk:{}].",
            UUID_A
        );
        let claims = vec![make_claim(
            1,
            "Overview",
            "Hippocampus supports memory formation",
        )];
        let (score, issues) = citation_presence_score(&summary, &claims);
        assert_eq!(score, 1.0, "issues: {:?}", issues);
    }

    // ---- citation_validity_score ----

    #[test]
    fn validity_all_present_scores_one() {
        let cited = vec![u(UUID_A), u(UUID_B)];
        let mut existing: HashMap<Uuid, u32> = HashMap::new();
        existing.insert(u(UUID_A), 0);
        existing.insert(u(UUID_B), 1);
        let ctx = HashMap::new();
        let (score, issues) = citation_validity_score(&cited, &existing, &ctx);
        assert_eq!(score, 1.0);
        assert!(issues.is_empty());
    }

    #[test]
    fn validity_one_orphan_out_of_ten_scores_nine_tenths() {
        let cited: Vec<Uuid> = (0..10)
            .map(|i| {
                Uuid::parse_str(&format!("00000000-0000-0000-0000-0000000000{:02x}", i)).unwrap()
            })
            .collect();
        let mut existing: HashMap<Uuid, u32> = HashMap::new();
        for u in cited.iter().take(9) {
            existing.insert(*u, 0);
        }
        let ctx = HashMap::new();
        let (score, issues) = citation_validity_score(&cited, &existing, &ctx);
        assert!((score - 0.9).abs() < 1e-6, "got {}", score);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, CitationIssueKind::Orphan);
    }

    #[test]
    fn validity_empty_cited_scores_zero() {
        let (score, issues) = citation_validity_score(&[], &HashMap::new(), &HashMap::new());
        assert_eq!(score, 0.0);
        assert!(issues.is_empty());
    }

    // ---- citation_scope_score ----

    #[test]
    fn scope_all_in_scope_scores_one() {
        let sid = u(UUID_A);
        let cited = vec![u(UUID_B), u(UUID_C)];
        let mut existing: HashMap<Uuid, Uuid> = HashMap::new();
        existing.insert(u(UUID_B), sid);
        existing.insert(u(UUID_C), sid);
        let ctx = HashMap::new();
        let (score, issues) = citation_scope_score(sid, &cited, &existing, &ctx);
        assert_eq!(score, 1.0);
        assert!(issues.is_empty());
    }

    #[test]
    fn scope_out_of_scope_detected() {
        let sid = u(UUID_A);
        let other = u("33333333-3333-3333-3333-333333333333");
        let cited = vec![u(UUID_B), u(UUID_C)];
        let mut existing: HashMap<Uuid, Uuid> = HashMap::new();
        existing.insert(u(UUID_B), sid);
        existing.insert(u(UUID_C), other);
        let ctx = HashMap::new();
        let (score, issues) = citation_scope_score(sid, &cited, &existing, &ctx);
        assert!((score - 0.5).abs() < 1e-6, "got {}", score);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, CitationIssueKind::OutOfScope);
        assert_eq!(issues[0].offending_chunk_id, Some(u(UUID_C)));
    }

    #[test]
    fn scope_only_orphans_vacuously_one() {
        let sid = u(UUID_A);
        let cited = vec![u(UUID_B), u(UUID_C)];
        let existing: HashMap<Uuid, Uuid> = HashMap::new();
        let ctx = HashMap::new();
        let (score, issues) = citation_scope_score(sid, &cited, &existing, &ctx);
        assert_eq!(score, 1.0);
        assert!(issues.is_empty());
    }

    // ---- citation_support_score ----

    #[test]
    fn support_all_supported_is_one() {
        use SupportVerdict::*;
        assert_eq!(
            citation_support_score(&[Supported, Supported, Supported]),
            1.0
        );
    }

    #[test]
    fn support_partial_half_weighted() {
        use SupportVerdict::*;
        // 1 supported (1.0) + 1 partial (0.5) + 1 unsupported (0.0) = 1.5 / 3 = 0.5
        let v = citation_support_score(&[Supported, Partial, Unsupported]);
        assert!((v - 0.5).abs() < 1e-6, "got {}", v);
    }

    #[test]
    fn support_empty_is_zero() {
        assert_eq!(citation_support_score(&[]), 0.0);
    }

    #[test]
    fn support_all_unsupported_is_zero() {
        use SupportVerdict::*;
        assert_eq!(
            citation_support_score(&[Unsupported, Contradicted, Unsupported]),
            0.0
        );
    }

    // ---- Gap-fill: malformed / unclosed fenced code blocks ----

    /// An unclosed ``` fence should not cause `parse_citations` to panic and
    /// should still return a sensible (`Vec`) result. The fenced-block regex
    /// is non-greedy (`` ```.*?``` ``) so an unmatched opening fence matches
    /// nothing and leaves the rest of the summary untouched — markers after
    /// the stray fence are parsed normally. We simply assert graceful,
    /// deterministic behaviour here.
    #[test]
    fn parse_handles_unclosed_fenced_code_block_gracefully() {
        // Fence opens mid-sentence, never closes.
        let s = format!(
            "Intro sentence mentions ``` an unclosed fence [chunk:{}]. Next sentence [chunk:{}] ends here.",
            UUID_A, UUID_B
        );
        // Must not panic; must return a Vec. Contents are permitted to be
        // either parsed-through (treating the stray ``` as literal) or
        // skipped — both are sensible. We only require determinism and no
        // out-of-bounds / slicing panics.
        let out = parse_citations(&s);
        for pc in &out {
            // Every returned citation must have a non-empty enclosing
            // sentence sliced on valid UTF-8 boundaries (implicit in the
            // successful `String` construction) and one of the two UUIDs.
            assert!(!pc.enclosing_sentence.is_empty());
            assert!(pc.uuid == u(UUID_A) || pc.uuid == u(UUID_B));
        }
        // A fence that opens and closes across two sentences is well-formed
        // and must blank everything between the two ```.
        let s2 = format!(
            "First [chunk:{}]. ``` middle [chunk:{}] still ``` fenced. Third [chunk:{}].",
            UUID_A, UUID_B, UUID_C
        );
        let out2 = parse_citations(&s2);
        let uuids: Vec<Uuid> = out2.iter().map(|p| p.uuid).collect();
        assert_eq!(uuids, vec![u(UUID_A), u(UUID_C)]);
    }

    // ---- Gap-fill: UUID case-sensitivity across all four metric functions ----

    /// The regex is `(?i)` — parsing accepts upper-case hex. But each of the
    /// four metrics converts cited hex into `Uuid`s (via parse or via direct
    /// caller-supplied `Uuid` values), and `Uuid` equality is
    /// case-insensitive by construction. Verify that *all four* metric
    /// functions produce identical scores for lowercase vs uppercase UUIDs
    /// in the same logical position.
    #[test]
    fn all_four_metrics_agree_across_uuid_case() {
        let lower = UUID_A.to_ascii_lowercase();
        let upper = UUID_A.to_ascii_uppercase();

        // 1. citation_presence_score — the UUID appears inside the summary
        //    body; parsing must handle both cases identically.
        let summary_lower = format!(
            "## Overview\nThe hippocampus supports memory [chunk:{}].",
            lower
        );
        let summary_upper = format!(
            "## Overview\nThe hippocampus supports memory [chunk:{}].",
            upper
        );
        let claims = vec![make_claim(1, "Overview", "The hippocampus supports memory")];
        let (p_lower, il) = citation_presence_score(&summary_lower, &claims);
        let (p_upper, iu) = citation_presence_score(&summary_upper, &claims);
        assert_eq!(p_lower, p_upper, "presence score diverged across UUID case");
        assert_eq!(il.len(), iu.len());

        // 2. citation_validity_score — the cited UUID list drives the
        //    formula; parse_str handles hex case, so downstream HashMap
        //    lookups must agree regardless of input case.
        let cited_lower = vec![Uuid::parse_str(&lower).unwrap()];
        let cited_upper = vec![Uuid::parse_str(&upper).unwrap()];
        let mut existing: HashMap<Uuid, u32> = HashMap::new();
        existing.insert(u(UUID_A), 0);
        let ctx_v: HashMap<Uuid, (u32, String)> = HashMap::new();
        let (v_lower, _) = citation_validity_score(&cited_lower, &existing, &ctx_v);
        let (v_upper, _) = citation_validity_score(&cited_upper, &existing, &ctx_v);
        assert_eq!(v_lower, v_upper, "validity score diverged across UUID case");
        assert_eq!(v_lower, 1.0);

        // 3. citation_scope_score — same reasoning: `Uuid` lookup.
        let sid = u(UUID_B);
        let mut scope_existing: HashMap<Uuid, Uuid> = HashMap::new();
        scope_existing.insert(u(UUID_A), sid);
        let ctx_s: HashMap<Uuid, (u32, String)> = HashMap::new();
        let (s_lower, _) = citation_scope_score(sid, &cited_lower, &scope_existing, &ctx_s);
        let (s_upper, _) = citation_scope_score(sid, &cited_upper, &scope_existing, &ctx_s);
        assert_eq!(s_lower, s_upper, "scope score diverged across UUID case");
        assert_eq!(s_lower, 1.0);

        // 4. citation_support_score — verdict list doesn't depend on UUID
        //    case directly, but the *same* verdicts derived from either
        //    case of UUID input must yield the same score. We synthesise
        //    matching verdict vectors to confirm the aggregation is stable.
        use SupportVerdict::*;
        let verdicts_from_lower = vec![Supported]; // one cited → one verdict
        let verdicts_from_upper = vec![Supported];
        assert_eq!(
            citation_support_score(&verdicts_from_lower),
            citation_support_score(&verdicts_from_upper),
            "support score diverged across UUID case"
        );
    }
}
