//! LLM rubric pipeline (Step 6).
//!
//! `judge_rubric_for_summary` makes one call to brainatlas's `/judge-rubric`
//! and produces five normalised metrics ready for the cache layer.
//! Normalisation: each 1–5 integer becomes `(score - 1) / 4` so the output
//! lands in `[0.0, 1.0]` like every other metric.

use crate::infra::{BrainatlasClient, SummaryRow};
use crate::ServiceError;
use backon::{ExponentialBuilder, Retryable};
use brainatlas_rpc_types::evals as brpc;
use domain::{EvalMetric, RubricCriterion, RubricScores};
use std::error::Error;

#[derive(Debug, Clone)]
pub struct RubricConfig {
    pub brainatlas_base_url: String,
    pub rubric_chat_model: String,
}

#[derive(Debug, Clone)]
pub struct RubricMetricEntry {
    pub metric: EvalMetric,
    pub score: f32,
    pub details: serde_json::Value,
    pub judge_model: String,
}

pub async fn judge_rubric_for_summary<BC, E>(
    brainatlas: &BC,
    summary: &SummaryRow,
    cfg: &RubricConfig,
) -> Result<Vec<RubricMetricEntry>, ServiceError<E>>
where
    BC: BrainatlasClient<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let req = brpc::JudgeRubricRequest {
        summary_text: summary.summary.clone(),
        region_name: summary.name.clone(),
        chat_model: Some(cfg.rubric_chat_model.clone()),
    };

    let policy = ExponentialBuilder::default()
        .with_min_delay(std::time::Duration::from_secs(1))
        .with_max_delay(std::time::Duration::from_secs(10))
        .with_max_times(3);

    let scores: RubricScores = (|| brainatlas.judge_rubric(&cfg.brainatlas_base_url, req.clone()))
        .retry(&policy)
        .await
        .map_err(ServiceError::InfraError)?;

    Ok(vec![
        entry(EvalMetric::RubricRelevance, &scores.relevance, &cfg.rubric_chat_model),
        entry(EvalMetric::RubricCoherence, &scores.coherence, &cfg.rubric_chat_model),
        entry(EvalMetric::RubricSpecificity, &scores.specificity, &cfg.rubric_chat_model),
        entry(
            EvalMetric::RubricClinicalUtility,
            &scores.clinical_utility,
            &cfg.rubric_chat_model,
        ),
        entry(EvalMetric::RubricTerminology, &scores.terminology, &cfg.rubric_chat_model),
    ])
}

fn entry(metric: EvalMetric, c: &RubricCriterion, judge_model: &str) -> RubricMetricEntry {
    RubricMetricEntry {
        metric,
        score: normalise_1_to_5(c.score),
        details: serde_json::json!({
            "raw_score": c.score,
            "rationale": c.rationale,
        }),
        judge_model: judge_model.to_string(),
    }
}

/// Map an integer 1..=5 onto `[0.0, 1.0]`. Out-of-range scores are clamped
/// before normalisation so a misbehaving judge can't produce > 1.0.
fn normalise_1_to_5(raw: u8) -> f32 {
    let clamped = raw.clamp(1, 5);
    (clamped as f32 - 1.0) / 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_endpoints() {
        assert_eq!(normalise_1_to_5(1), 0.0);
        assert_eq!(normalise_1_to_5(5), 1.0);
        assert!((normalise_1_to_5(3) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_scores_are_clamped() {
        assert_eq!(normalise_1_to_5(0), 0.0);
        assert_eq!(normalise_1_to_5(99), 1.0);
    }
}
