//! Persistent eval domain types: scores, runs, metric enumeration.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr};
use uuid::Uuid;

/// One persisted score row in `eval_scores`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalScore {
    pub id: Uuid,
    pub summary_id: Uuid,
    /// SHA-256 of the summary text at score time. Cache key.
    pub summary_hash: String,
    /// Free-form metric name (matches `EvalMetric::as_static_str` for known metrics).
    pub metric: String,
    /// Normalized score in [0.0, 1.0].
    pub score: f32,
    pub judge_model: Option<String>,
    pub details: Option<serde_json::Value>,
    pub eval_version: String,
    pub created_at: NaiveDateTime,
}

/// New `eval_scores` row to insert.
#[derive(Debug, Clone)]
pub struct NewEvalScore {
    pub summary_id: Uuid,
    pub summary_hash: String,
    pub metric: String,
    pub score: f32,
    pub judge_model: Option<String>,
    pub details: Option<serde_json::Value>,
    pub eval_version: String,
}

/// Lifecycle status of a per-summary eval run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, IntoStaticStr, Serialize, Deserialize)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EvalRunStatus {
    Queued,
    Running,
    Complete,
    Failed,
}

/// One persisted lifecycle row in `eval_runs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRun {
    pub id: Uuid,
    pub summary_id: Uuid,
    pub eval_version: String,
    pub status: EvalRunStatus,
    pub error_message: Option<String>,
    pub started_at: Option<NaiveDateTime>,
    pub completed_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

/// Enumeration of every metric the eval system knows how to produce.
///
/// `IntoStaticStr` (snake_case) gives the exact string written to
/// `eval_scores.metric`. Add a variant here whenever a new metric ships;
/// `EnumString` makes it cheap to round-trip from DB rows back to typed enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum EvalMetric {
    // ---- Structural (deterministic, no LLM) ----
    SectionCompleteness,
    LengthInRange,
    AcronymMention,
    NoPlaceholderText,

    // ---- Groundedness (LLM + retrieval) ----
    ClaimGroundedness,
    HallucinationRate,

    // ---- Rubric (LLM) ----
    RubricRelevance,
    RubricCoherence,
    RubricSpecificity,
    RubricClinicalUtility,
    RubricTerminology,

    // ---- Citation (deterministic + optional LLM support judge) ----
    CitationPresence,
    CitationValidity,
    CitationScope,
    CitationSupport,
}

impl EvalMetric {
    /// All metric variants in their canonical reporting order.
    pub fn all() -> &'static [EvalMetric] {
        use EvalMetric::*;
        &[
            SectionCompleteness,
            LengthInRange,
            AcronymMention,
            NoPlaceholderText,
            ClaimGroundedness,
            HallucinationRate,
            RubricRelevance,
            RubricCoherence,
            RubricSpecificity,
            RubricClinicalUtility,
            RubricTerminology,
            CitationPresence,
            CitationValidity,
            CitationScope,
            CitationSupport,
        ]
    }

    /// Static string used as the DB column value.
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_string_round_trip() {
        for m in EvalMetric::all() {
            let s: &'static str = (*m).into();
            let back: EvalMetric = s.parse().unwrap();
            assert_eq!(back, *m);
        }
    }

    #[test]
    fn metric_strings_are_snake_case() {
        assert_eq!(EvalMetric::SectionCompleteness.as_str(), "section_completeness");
        assert_eq!(EvalMetric::ClaimGroundedness.as_str(), "claim_groundedness");
        assert_eq!(EvalMetric::RubricClinicalUtility.as_str(), "rubric_clinical_utility");
        assert_eq!(EvalMetric::CitationPresence.as_str(), "citation_presence");
        assert_eq!(EvalMetric::CitationValidity.as_str(), "citation_validity");
        assert_eq!(EvalMetric::CitationScope.as_str(), "citation_scope");
        assert_eq!(EvalMetric::CitationSupport.as_str(), "citation_support");
    }

    #[test]
    fn metric_all_covers_fifteen_metrics() {
        // 11 legacy + 4 new citation metrics = 15.
        assert_eq!(EvalMetric::all().len(), 15);
    }

    #[test]
    fn run_status_serialises_snake_case() {
        let s = serde_json::to_string(&EvalRunStatus::Complete).unwrap();
        assert_eq!(s, "\"complete\"");
        let parsed: EvalRunStatus = serde_json::from_str("\"failed\"").unwrap();
        assert_eq!(parsed, EvalRunStatus::Failed);
    }
}
