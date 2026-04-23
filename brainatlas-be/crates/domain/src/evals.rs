//! Eval-related domain types: claim extraction, groundedness judgments, rubric scores.
//!
//! These types are the wire contract between brainatlas-be (which runs the LLM)
//! and evals-be (which orchestrates eval pipelines and persists scores). They live
//! in the `domain` crate so both sides depend on the same definitions.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

// -------- Claim extraction --------

/// A single atomic factual claim extracted from a summary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Claim {
    /// Sequential claim number assigned by the extractor (1-based).
    pub id: u32,
    /// The `## Section Heading` (without the `## ` prefix) that contained the claim,
    /// or `"Preamble"` if the claim sits before any heading.
    pub section: String,
    /// The claim text, in plain English, without `[chunk:...]` citation markers.
    pub text: String,
    /// UUIDs extracted from `[chunk:<uuid>]` markers that appeared alongside
    /// this claim in the original summary text. Empty if the claim was not
    /// cited. Optional for backward compatibility with old cached payloads.
    ///
    /// Deserialization is lenient: any element that is not a syntactically
    /// valid UUID string is dropped silently. This protects the eval
    /// pipeline from LLM-output drift where the model occasionally returns
    /// malformed identifiers (e.g. truncated UUIDs, paper-IDs, free text)
    /// instead of clean chunk UUIDs. The resulting `Vec<Uuid>` only ever
    /// contains entries that round-trip through `Uuid::parse_str`.
    #[serde(default, deserialize_with = "deserialize_lenient_uuid_vec")]
    #[schemars(with = "Vec<String>")]
    pub cited_chunks: Vec<Uuid>,
}

/// Custom deserializer for `Vec<Uuid>` that silently skips elements which
/// cannot be parsed as a UUID. Accepts a JSON array of strings.
fn deserialize_lenient_uuid_vec<'de, D>(deserializer: D) -> Result<Vec<Uuid>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: Vec<String> = Vec::deserialize(deserializer)?;
    Ok(raw.into_iter().filter_map(|s| Uuid::parse_str(s.trim()).ok()).collect())
}

/// Top-level response from the claim-extraction prompt.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ClaimsResponse {
    pub claims: Vec<Claim>,
}

// -------- Groundedness judgement --------

/// Verdict assigned by the judge LLM for a single claim against retrieved evidence.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GroundednessLabel {
    Supported,
    Partial,
    Contradicted,
    Unsupported,
}

/// Single judgement produced by `judge_groundedness`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroundednessVerdict {
    pub verdict: GroundednessLabel,
    /// Judge confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// 1-based indices into the evidence list passed to the judge.
    #[serde(default)]
    pub supporting_chunks: Vec<u32>,
    /// One-sentence rationale.
    #[serde(default)]
    pub rationale: String,
}

// -------- Rubric scoring --------

/// Single criterion score plus rationale, on a 1-5 integer scale.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RubricCriterion {
    /// Integer score in `[1, 5]`.
    pub score: u8,
    /// One-sentence rationale.
    #[serde(default)]
    pub rationale: String,
}

/// Five fixed rubric criteria scored by the rubric judge.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RubricScores {
    pub relevance: RubricCriterion,
    pub coherence: RubricCriterion,
    pub specificity: RubricCriterion,
    pub clinical_utility: RubricCriterion,
    pub terminology: RubricCriterion,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_response_roundtrips() {
        let resp = ClaimsResponse {
            claims: vec![Claim {
                id: 1,
                section: "Overview".to_string(),
                text: "The hippocampus supports declarative memory.".to_string(),
                cited_chunks: vec![Uuid::nil()],
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ClaimsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.claims.len(), 1);
        assert_eq!(back.claims[0].text, resp.claims[0].text);
        assert_eq!(back.claims[0].cited_chunks, resp.claims[0].cited_chunks);
    }

    /// Legacy payloads (pre-citation-evals) must still deserialize as claims
    /// with an empty `cited_chunks` list.
    #[test]
    fn claims_response_tolerates_missing_cited_chunks() {
        let json = r#"{"claims":[{"id":1,"section":"Overview","text":"The hippocampus supports declarative memory."}]}"#;
        let parsed: ClaimsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.claims.len(), 1);
        assert!(parsed.claims[0].cited_chunks.is_empty());
    }

    /// Lenient deserialization: malformed UUID entries in `cited_chunks` are
    /// silently dropped instead of failing the entire response. Real LLMs
    /// occasionally produce truncated UUIDs, paper-IDs, or free text in this
    /// field; we want the eval pipeline to keep moving.
    #[test]
    fn claims_response_drops_invalid_uuids_in_cited_chunks() {
        let json = r#"{
            "claims": [{
                "id": 1,
                "section": "Overview",
                "text": "Mixed valid and invalid IDs.",
                "cited_chunks": [
                    "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
                    "not-a-uuid",
                    "https://example.com/foo",
                    "b73abca3-a7a0-468a-9646-6794527cdd71",
                    ""
                ]
            }]
        }"#;
        let parsed: ClaimsResponse = serde_json::from_str(json).expect("must not error");
        assert_eq!(parsed.claims.len(), 1);
        let chunks = &parsed.claims[0].cited_chunks;
        assert_eq!(chunks.len(), 2, "exactly the two well-formed UUIDs survive");
        assert_eq!(
            chunks[0].to_string(),
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
        );
        assert_eq!(
            chunks[1].to_string(),
            "b73abca3-a7a0-468a-9646-6794527cdd71"
        );
    }

    #[test]
    fn groundedness_label_serialises_lowercase() {
        let v = GroundednessVerdict {
            verdict: GroundednessLabel::Supported,
            confidence: 0.9,
            supporting_chunks: vec![1, 3],
            rationale: "Direct match".to_string(),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("\"verdict\":\"supported\""));
    }

    #[test]
    fn rubric_scores_parse_full_payload() {
        let json = r#"{
            "relevance":        {"score": 5, "rationale": "stays on topic"},
            "coherence":        {"score": 4, "rationale": "mostly clear"},
            "specificity":      {"score": 3, "rationale": "some vague spots"},
            "clinical_utility": {"score": 5, "rationale": "actionable"},
            "terminology":      {"score": 4, "rationale": "modern usage"}
        }"#;
        let parsed: RubricScores = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.relevance.score, 5);
        assert_eq!(parsed.terminology.rationale, "modern usage");
    }

    // ---------- Gap-fill tests (Plan Task 1.12: evals.rs) ----------

    /// Every `GroundednessLabel` variant must survive a serde round-trip
    /// against its documented lowercase wire spelling. The enum has no
    /// `Display`/`FromStr`, so this is the only string contract we need to
    /// pin.
    #[test]
    fn groundedness_label_every_variant_roundtrips() {
        let cases: &[(GroundednessLabel, &str)] = &[
            (GroundednessLabel::Supported, "\"supported\""),
            (GroundednessLabel::Partial, "\"partial\""),
            (GroundednessLabel::Contradicted, "\"contradicted\""),
            (GroundednessLabel::Unsupported, "\"unsupported\""),
        ];
        for (label, wire) in cases {
            let serialised = serde_json::to_string(label).unwrap();
            assert_eq!(serialised, *wire, "wire form for {:?}", label);
            let back: GroundednessLabel = serde_json::from_str(&serialised).unwrap();
            assert_eq!(back, *label, "round-trip for {:?}", label);
        }
    }

    /// A full `GroundednessVerdict` with each label variant must survive a
    /// `to_value -> from_value` round-trip with all fields preserved.
    #[test]
    fn groundedness_verdict_roundtrips_for_every_label() {
        for label in [
            GroundednessLabel::Supported,
            GroundednessLabel::Partial,
            GroundednessLabel::Contradicted,
            GroundednessLabel::Unsupported,
        ] {
            let v = GroundednessVerdict {
                verdict: label,
                confidence: 0.75,
                supporting_chunks: vec![1, 2, 4],
                rationale: "because reasons".to_string(),
            };
            let value = serde_json::to_value(&v).unwrap();
            let back: GroundednessVerdict = serde_json::from_value(value).unwrap();
            assert_eq!(back.verdict, v.verdict);
            assert!((back.confidence - v.confidence).abs() < 1e-6);
            assert_eq!(back.supporting_chunks, v.supporting_chunks);
            assert_eq!(back.rationale, v.rationale);
        }
    }
}
