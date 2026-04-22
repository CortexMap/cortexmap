//! Deterministic, no-LLM structural metrics.
//!
//! Each function is a pure `&str -> f32` mapping with output in `[0.0, 1.0]`.
//! Tests at the bottom of this file lock the formula — treat them as the
//! specification.

/// Required section headings any well-formed summary should contain. The score
/// is the fraction of these headings present in the summary text.
const REQUIRED_SECTIONS: &[&str] = &[
    "## Overview",
    "## Anatomy & Connectivity",
    "## Functions",
    "## Associated Disorders",
    "## Symptoms of Damage or Dysfunction",
    "## Research Highlights",
];

/// Placeholder strings that indicate the LLM bailed or a draft slipped
/// through. Any one match → score 0.0.
const PLACEHOLDER_NEEDLES: &[&str] = &[
    "TBD",
    "TODO",
    "Lorem ipsum",
    "[placeholder]",
    "(to be filled)",
];

const LENGTH_LOW: usize = 1500;
const LENGTH_HIGH: usize = 10_000;
const LENGTH_FLOOR: usize = 0;
const LENGTH_CEIL: usize = 20_000;

/// Fraction of the 6 required section headings present (case-sensitive match).
pub fn section_completeness(summary: &str) -> f32 {
    let total = REQUIRED_SECTIONS.len() as f32;
    let found = REQUIRED_SECTIONS
        .iter()
        .filter(|h| summary.contains(*h))
        .count() as f32;
    found / total
}

/// 1.0 when length sits in `[LENGTH_LOW, LENGTH_HIGH]`. Linear falloff to 0.0
/// at length `0` (below) or `LENGTH_CEIL` (above). Anything beyond `LENGTH_CEIL`
/// also scores 0.0.
pub fn length_in_range(summary: &str) -> f32 {
    let len = summary.len();
    if (LENGTH_LOW..=LENGTH_HIGH).contains(&len) {
        return 1.0;
    }
    if len < LENGTH_LOW {
        if len == LENGTH_FLOOR {
            return 0.0;
        }
        return (len as f32 - LENGTH_FLOOR as f32) / (LENGTH_LOW as f32 - LENGTH_FLOOR as f32);
    }
    // len > LENGTH_HIGH
    if len >= LENGTH_CEIL {
        return 0.0;
    }
    1.0 - ((len as f32 - LENGTH_HIGH as f32) / (LENGTH_CEIL as f32 - LENGTH_HIGH as f32))
}

/// Score for acronym presence in the summary text.
///
/// Matching strategy (first match wins, highest score returned):
///  1. **Exact substring** (case-insensitive) → `1.0`
///  2. **Dotted / hyphenated form** — e.g. `HPC` matches `H.P.C.` or `H-P-C` → `1.0`
///  3. **Fuzzy word match** — any whitespace-delimited token within Levenshtein
///     distance ≤ `max(1, acronym.len() / 3)` of the acronym → `0.5`
///
/// If `acronym` is `None` or blank, returns `1.0` (vacuously satisfied).
pub fn acronym_mention(summary: &str, acronym: Option<&str>) -> f32 {
    match acronym {
        None => 1.0,
        Some(a) => {
            let a = a.trim();
            if a.is_empty() {
                return 1.0;
            }
            let summary_lower = summary.to_ascii_lowercase();
            let acronym_lower = a.to_ascii_lowercase();

            // 1. Exact case-insensitive substring.
            if summary_lower.contains(&acronym_lower) {
                return 1.0;
            }

            // 2. Dotted / hyphenated form (e.g. "H.P.C." or "H-P-C" for "HPC").
            if acronym_lower.len() >= 2 {
                let dotted: String = acronym_lower
                    .chars()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join("[.\\-]?");
                if let Ok(re) = regex::Regex::new(&format!("(?i){dotted}")) {
                    if re.is_match(summary) {
                        return 1.0;
                    }
                }
            }

            // 3. Fuzzy word-level match via Levenshtein distance.
            let max_dist = (acronym_lower.len() / 3).max(1);
            for token in summary_lower.split_whitespace() {
                // Strip common trailing punctuation so "HPC," or "(HPC)" don't
                // inflate edit distance.
                let token = token.trim_matches(|c: char| !c.is_alphanumeric());
                if token.is_empty() {
                    continue;
                }
                // Skip tokens whose length differs too much — they can never be
                // within `max_dist`.
                let len_diff = token.len().abs_diff(acronym_lower.len());
                if len_diff > max_dist {
                    continue;
                }
                if strsim::levenshtein(token, &acronym_lower) <= max_dist {
                    return 0.5;
                }
            }

            0.0
        }
    }
}

/// 0.0 if any placeholder needle appears (case-sensitive), else 1.0.
pub fn no_placeholder_text(summary: &str) -> f32 {
    if PLACEHOLDER_NEEDLES.iter().any(|n| summary.contains(*n)) {
        0.0
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fully_formed_summary() -> String {
        REQUIRED_SECTIONS
            .iter()
            .map(|h| format!("{}\nbody text.\n\n", h))
            .collect::<String>()
    }

    #[test]
    fn section_completeness_full_score() {
        assert_eq!(section_completeness(&fully_formed_summary()), 1.0);
    }

    #[test]
    fn section_completeness_partial_score() {
        let s = "## Overview\nfoo\n\n## Functions\nbar";
        let v = section_completeness(s);
        // 2 of 6 → 0.333…
        assert!((v - 2.0_f32 / 6.0).abs() < 1e-6);
    }

    #[test]
    fn section_completeness_empty_summary_is_zero() {
        assert_eq!(section_completeness(""), 0.0);
    }

    #[test]
    fn length_in_range_inside_window() {
        let s = "x".repeat(5000);
        assert_eq!(length_in_range(&s), 1.0);
    }

    #[test]
    fn length_in_range_at_low_boundary() {
        let s = "x".repeat(LENGTH_LOW);
        assert_eq!(length_in_range(&s), 1.0);
    }

    #[test]
    fn length_in_range_at_high_boundary() {
        let s = "x".repeat(LENGTH_HIGH);
        assert_eq!(length_in_range(&s), 1.0);
    }

    #[test]
    fn length_in_range_below_window_falls_off_linearly() {
        let s = "x".repeat(LENGTH_LOW / 2);
        let v = length_in_range(&s);
        assert!((v - 0.5).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn length_in_range_above_window_falls_off_linearly() {
        // half-way between HIGH and CEIL → 0.5
        let mid = (LENGTH_HIGH + LENGTH_CEIL) / 2;
        let s = "x".repeat(mid);
        let v = length_in_range(&s);
        assert!((v - 0.5).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn length_in_range_empty_is_zero() {
        assert_eq!(length_in_range(""), 0.0);
    }

    #[test]
    fn length_in_range_giant_is_zero() {
        let s = "x".repeat(LENGTH_CEIL + 1);
        assert_eq!(length_in_range(&s), 0.0);
    }

    #[test]
    fn acronym_mention_present() {
        assert_eq!(
            acronym_mention("The HPC supports memory.", Some("HPC")),
            1.0
        );
    }

    #[test]
    fn acronym_mention_case_insensitive() {
        assert_eq!(
            acronym_mention("The hpc supports memory.", Some("HPC")),
            1.0
        );
        assert_eq!(
            acronym_mention("The HPC supports memory.", Some("hpc")),
            1.0
        );
    }

    #[test]
    fn acronym_mention_trims_whitespace() {
        assert_eq!(
            acronym_mention("The HPC supports memory.", Some(" HPC ")),
            1.0
        );
        assert_eq!(
            acronym_mention("The HPC supports memory.", Some("  HPC\t")),
            1.0
        );
    }

    #[test]
    fn acronym_mention_dotted_form() {
        assert_eq!(
            acronym_mention("The H.P.C. supports memory.", Some("HPC")),
            1.0
        );
        assert_eq!(
            acronym_mention("The H-P-C supports memory.", Some("HPC")),
            1.0
        );
    }

    #[test]
    fn acronym_mention_fuzzy_one_char_off() {
        // "HPC" vs "HRC" — edit distance 1, within threshold max(1, 3/3)=1
        assert_eq!(
            acronym_mention("The HRC supports memory.", Some("HPC")),
            0.5
        );
    }

    #[test]
    fn acronym_mention_fuzzy_too_distant() {
        // "HPC" vs "XYZ" — edit distance 3, exceeds threshold
        assert_eq!(
            acronym_mention("The XYZ supports memory.", Some("HPC")),
            0.0
        );
    }

    #[test]
    fn acronym_mention_fuzzy_with_punctuation() {
        // Token "(HRC)" should be stripped to "HRC" then fuzzy-matched
        assert_eq!(
            acronym_mention("The (HRC) supports memory.", Some("HPC")),
            0.5
        );
    }

    #[test]
    fn acronym_mention_absent() {
        assert_eq!(acronym_mention("memory region", Some("HPC")), 0.0);
    }

    #[test]
    fn acronym_mention_none_is_full_score() {
        assert_eq!(acronym_mention("anything", None), 1.0);
        assert_eq!(acronym_mention("anything", Some("")), 1.0);
        assert_eq!(acronym_mention("anything", Some("   ")), 1.0);
    }

    #[test]
    fn no_placeholder_text_clean() {
        assert_eq!(no_placeholder_text("clean summary text"), 1.0);
    }

    #[test]
    fn no_placeholder_text_each_needle_trips_score() {
        for needle in PLACEHOLDER_NEEDLES {
            let s = format!("ok ok {needle} more text");
            assert_eq!(no_placeholder_text(&s), 0.0, "needle={needle}");
        }
    }
}
