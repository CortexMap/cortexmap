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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, IntoStaticStr, Serialize, Deserialize,
)]
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

    // ---- Rubric gated by groundedness (derived, no LLM) ----
    //
    // Each `rubric_*_gated = rubric_* * claim_groundedness`.
    // The rubric judge sees no source chunks, so it grades prose style
    // conditional on the prose being trusted. Multiplying by groundedness
    // converts "writing quality" into "writing quality weighted by whether
    // the facts are grounded in evidence", preventing confidently-wrong
    // summaries from hiding behind a near-1.0 rubric score.
    RubricRelevanceGated,
    RubricCoherenceGated,
    RubricSpecificityGated,
    RubricClinicalUtilityGated,
    RubricTerminologyGated,

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
            RubricRelevanceGated,
            RubricCoherenceGated,
            RubricSpecificityGated,
            RubricClinicalUtilityGated,
            RubricTerminologyGated,
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
    // These types are re-exported at the crate root (see lib.rs) from
    // `brainatlas_domain`; we import them here so the round-trip tests
    // below don't need to qualify them every time.
    use crate::{GroundednessLabel, RubricCriterion, RubricScores};

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
        assert_eq!(
            EvalMetric::SectionCompleteness.as_str(),
            "section_completeness"
        );
        assert_eq!(EvalMetric::ClaimGroundedness.as_str(), "claim_groundedness");
        assert_eq!(
            EvalMetric::RubricClinicalUtility.as_str(),
            "rubric_clinical_utility"
        );
        assert_eq!(EvalMetric::CitationPresence.as_str(), "citation_presence");
        assert_eq!(EvalMetric::CitationValidity.as_str(), "citation_validity");
        assert_eq!(EvalMetric::CitationScope.as_str(), "citation_scope");
        assert_eq!(EvalMetric::CitationSupport.as_str(), "citation_support");
        assert_eq!(
            EvalMetric::RubricRelevanceGated.as_str(),
            "rubric_relevance_gated"
        );
        assert_eq!(
            EvalMetric::RubricClinicalUtilityGated.as_str(),
            "rubric_clinical_utility_gated"
        );
        assert_eq!(
            EvalMetric::RubricTerminologyGated.as_str(),
            "rubric_terminology_gated"
        );
    }

    #[test]
    fn metric_all_covers_twenty_metrics() {
        // 11 legacy + 4 citation + 5 gated rubric = 20.
        assert_eq!(EvalMetric::all().len(), 20);
    }

    #[test]
    fn run_status_serialises_snake_case() {
        let s = serde_json::to_string(&EvalRunStatus::Complete).unwrap();
        assert_eq!(s, "\"complete\"");
        let parsed: EvalRunStatus = serde_json::from_str("\"failed\"").unwrap();
        assert_eq!(parsed, EvalRunStatus::Failed);
    }

    /// Every `EvalRunStatus` variant round-trips through serde_json. If
    /// someone adds a variant and forgets `serde(rename_all = snake_case)`,
    /// the DB column value stops matching the enum wire name and this
    /// test catches it.
    #[test]
    fn run_status_round_trips_all_variants() {
        for status in [
            EvalRunStatus::Queued,
            EvalRunStatus::Running,
            EvalRunStatus::Complete,
            EvalRunStatus::Failed,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: EvalRunStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status, "round-trip failed for {:?}", status);
            // strum Display must also match the serde wire form.
            let display = format!("{}", status);
            assert_eq!(
                json.trim_matches('"'),
                display,
                "serde form diverged from Display for {:?}",
                status
            );
        }
    }

    /// Every `EvalMetric` variant must round-trip via its static string.
    /// (`EvalMetric` is not Serde-serialised — it's written as a string
    /// into `eval_scores.metric` — so the round-trip goes through the
    /// `IntoStaticStr` / `EnumString` pair.) Also covers every Citation*
    /// variant that was added in the evals-v0.3 work.
    #[test]
    fn every_eval_metric_round_trips_via_static_str() {
        for &m in EvalMetric::all() {
            let s = m.as_str();
            let back: EvalMetric = s.parse().unwrap();
            assert_eq!(back, m, "round-trip failed for {:?}", m);
            // Display must match the DB-side static form too.
            assert_eq!(format!("{}", m), s);
        }
    }

    /// `EvalMetric::all()` must be deduplicated — duplicate variants would
    /// produce duplicate score rows and break the per-metric aggregate math.
    #[test]
    fn eval_metric_all_has_no_duplicates() {
        // EvalMetric is `Eq` but not `Hash`, so compare via the static str
        // (which is stable under `EnumString` / `IntoStaticStr`).
        let mut seen: Vec<&'static str> = Vec::new();
        for &m in EvalMetric::all() {
            let s = m.as_str();
            assert!(
                !seen.contains(&s),
                "duplicate variant in EvalMetric::all(): {}",
                s
            );
            seen.push(s);
        }
    }

    /// Unknown metric strings must fail to parse — never silently alias to
    /// a known variant.
    #[test]
    fn unknown_metric_string_errors() {
        assert!("not_a_metric".parse::<EvalMetric>().is_err());
        assert!("".parse::<EvalMetric>().is_err());
        assert!("SectionCompleteness".parse::<EvalMetric>().is_err());
    }

    /// Every `GroundednessLabel` variant must round-trip through serde_json,
    /// and the serialised form must be bare lowercase (the `serde(rename_all
    /// = "lowercase")` contract — the brainatlas judge prompt expects
    /// exactly these spellings).
    #[test]
    fn groundedness_label_round_trips_every_variant() {
        use GroundednessLabel::*;
        let cases = [
            (Supported, "\"supported\""),
            (Partial, "\"partial\""),
            (Contradicted, "\"contradicted\""),
            (Unsupported, "\"unsupported\""),
        ];
        for (label, expected_json) in cases {
            let json = serde_json::to_string(&label).unwrap();
            assert_eq!(json, expected_json, "bad serialisation for {:?}", label);
            let back: GroundednessLabel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, label);
        }
    }

    /// A `RubricCriterion` must survive serde round-trip with its score
    /// and rationale intact — and the default `rationale = ""` fallback
    /// must kick in when the field is absent in the incoming JSON.
    #[test]
    fn rubric_criterion_round_trips() {
        let original = RubricCriterion {
            score: 4,
            rationale: "mostly clear, some rough edges".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: RubricCriterion = serde_json::from_str(&json).unwrap();
        assert_eq!(back.score, original.score);
        assert_eq!(back.rationale, original.rationale);

        // Missing rationale should default to empty string.
        let sparse: RubricCriterion =
            serde_json::from_str(r#"{"score": 3}"#).expect("rationale is #[serde(default)]");
        assert_eq!(sparse.score, 3);
        assert!(sparse.rationale.is_empty());
    }

    /// A full `RubricScores` payload round-trips — all 5 criteria names
    /// must deserialise to the documented fields.
    #[test]
    fn rubric_scores_round_trip_full_payload() {
        let scores = RubricScores {
            relevance: RubricCriterion {
                score: 5,
                rationale: "r".to_string(),
            },
            coherence: RubricCriterion {
                score: 4,
                rationale: "c".to_string(),
            },
            specificity: RubricCriterion {
                score: 3,
                rationale: "s".to_string(),
            },
            clinical_utility: RubricCriterion {
                score: 2,
                rationale: "u".to_string(),
            },
            terminology: RubricCriterion {
                score: 1,
                rationale: "t".to_string(),
            },
        };
        let json = serde_json::to_string(&scores).unwrap();
        // Wire keys must be snake_case to match the brainatlas contract.
        assert!(json.contains("\"clinical_utility\""));
        let back: RubricScores = serde_json::from_str(&json).unwrap();
        assert_eq!(back.relevance.score, 5);
        assert_eq!(back.coherence.rationale, "c");
        assert_eq!(back.terminology.score, 1);
    }

    /// `EvalScore` persists to `eval_scores` with `details: Option<Value>`
    /// and must round-trip losslessly — including a `null` details column
    /// and a structured JSON object.
    #[test]
    fn eval_score_round_trips_with_and_without_details() {
        let id = Uuid::new_v4();
        let summary_id = Uuid::new_v4();
        let row_with = EvalScore {
            id,
            summary_id,
            summary_hash: "abc".to_string(),
            metric: EvalMetric::ClaimGroundedness.as_str().to_string(),
            score: 0.87,
            judge_model: Some("openai/gpt-4o-mini".to_string()),
            details: Some(serde_json::json!({"n_claims": 5, "supported": 4})),
            eval_version: "v0.3.0".to_string(),
            created_at: NaiveDateTime::default(),
        };
        let json = serde_json::to_string(&row_with).unwrap();
        let back: EvalScore = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, id);
        assert_eq!(back.metric, "claim_groundedness");
        assert_eq!(back.judge_model.as_deref(), Some("openai/gpt-4o-mini"));
        assert_eq!(
            back.details
                .as_ref()
                .and_then(|v| v.get("n_claims").and_then(|n| n.as_u64())),
            Some(5)
        );

        let row_without = EvalScore {
            details: None,
            judge_model: None,
            ..row_with
        };
        let json2 = serde_json::to_string(&row_without).unwrap();
        let back2: EvalScore = serde_json::from_str(&json2).unwrap();
        assert!(back2.details.is_none());
        assert!(back2.judge_model.is_none());
    }
}
