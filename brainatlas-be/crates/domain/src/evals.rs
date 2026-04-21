//! Eval-related domain types: claim extraction, groundedness judgments, rubric scores.
//!
//! These types are the wire contract between brainatlas-be (which runs the LLM)
//! and evals-be (which orchestrates eval pipelines and persists scores). They live
//! in the `domain` crate so both sides depend on the same definitions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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
    #[serde(default)]
    #[schemars(with = "Vec<String>")]
    pub cited_chunks: Vec<Uuid>,
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
}
