//! Per-summary scoring pipeline. Single entry point for both manual scoring
//! (HTTP `POST /api/evals/score`) and the orch poller.
//!
//! Order of execution:
//!  1. Structural metrics (free, no LLM).
//!  2. Groundedness (LLM + retrieval).
//!  3. Rubric (LLM, single call → 5 metrics).
//!
//! Each metric goes through `services::cache::score_with_cache` so identical
//! summary text never recomputes. A late LLM failure never loses an earlier
//! metric: each row is committed independently via the cache helper.

use crate::error::AppError;
use crate::EvalRuntimeConfig;
use chrono::Utc;
use domain::{compute_hash, EvalMetric, EvalRunStatus};
use services::{
    groundedness::{judge_groundedness_for_summary, GroundednessConfig},
    rubric::{judge_rubric_for_summary, RubricConfig},
    structural, BrainatlasClient, CachedScore, ComputedScore, EvalsDatabase, ServiceError,
};
use std::error::Error;
use uuid::Uuid;

/// Per-metric result returned to the HTTP layer.
#[derive(Debug, Clone)]
pub struct MetricOutcome {
    pub metric: String,
    pub score: f32,
    pub cached: bool,
    pub judge_model: Option<String>,
}

impl From<CachedScore> for MetricOutcome {
    fn from(c: CachedScore) -> Self {
        Self {
            metric: c.row.metric,
            score: c.row.score,
            cached: c.cached,
            judge_model: c.row.judge_model,
        }
    }
}

/// Score one summary across every known metric. Always tries every metric;
/// LLM failures degrade to skipping that metric (the run is recorded as
/// `failed` with the message but earlier successes are kept).
pub async fn run_all_metrics<DB, BC, E>(
    db: &DB,
    brainatlas: &BC,
    cfg: &EvalRuntimeConfig,
    summary_id: Uuid,
    eval_version: &str,
) -> Result<Vec<MetricOutcome>, AppError<E>>
where
    DB: EvalsDatabase<Error = E>,
    BC: BrainatlasClient<Error = E>,
    E: Error + Send + Sync + 'static,
{
    // Mark the run as running.
    let _ = db
        .upsert_run(
            &cfg.database_url,
            summary_id,
            eval_version,
            EvalRunStatus::Running,
            None,
        )
        .await;

    let summary = db
        .get_summary(&cfg.database_url, summary_id)
        .await
        .map_err(ServiceError::InfraError)?
        .ok_or(AppError::SummaryNotFound)?;

    let summary_hash = compute_hash(&summary.summary);
    let mut outcomes: Vec<MetricOutcome> = Vec::new();
    let mut first_error: Option<String> = None;

    // -------- Structural --------
    let summary_text = summary.summary.clone();
    let acronym = summary.acronym.clone();

    macro_rules! run_structural {
        ($metric:expr, $compute:expr) => {{
            let res = services::score_with_cache(
                db,
                &cfg.database_url,
                summary_id,
                &summary_hash,
                $metric.as_str(),
                eval_version,
                || async { Ok(ComputedScore::structural($compute)) },
            )
            .await;
            match res {
                Ok(c) => outcomes.push(c.into()),
                Err(e) => {
                    tracing::error!(metric = $metric.as_str(), error = %e, "structural metric failed");
                    if first_error.is_none() {
                        first_error = Some(format!("{}: {}", $metric.as_str(), e));
                    }
                }
            }
        }};
    }

    let st_text = summary_text.clone();
    run_structural!(
        EvalMetric::SectionCompleteness,
        structural::section_completeness(&st_text)
    );
    let st_text = summary_text.clone();
    run_structural!(
        EvalMetric::LengthInRange,
        structural::length_in_range(&st_text)
    );
    let st_text = summary_text.clone();
    let st_acr = acronym.clone();
    run_structural!(
        EvalMetric::AcronymMention,
        structural::acronym_mention(&st_text, st_acr.as_deref())
    );
    let st_text = summary_text.clone();
    run_structural!(
        EvalMetric::NoPlaceholderText,
        structural::no_placeholder_text(&st_text)
    );

    // -------- Groundedness (2 metrics from one pipeline) --------
    let g_cfg = GroundednessConfig {
        brainatlas_base_url: cfg.brainatlas_base_url.clone(),
        judge_chat_model: cfg.judge_chat_model.clone(),
        embedding_model: cfg.embedding_model.clone(),
        top_k_chunks: cfg.top_k_chunks,
        similarity_threshold: cfg.similarity_threshold,
    };

    // Probe the cache first: if BOTH metrics already have rows for this
    // (summary_hash, eval_version), skip the entire LLM pipeline.
    let g_cached = db
        .lookup_score_by_hash(
            &cfg.database_url,
            &summary_hash,
            EvalMetric::ClaimGroundedness.as_str(),
            eval_version,
        )
        .await
        .map_err(ServiceError::InfraError)?;
    let h_cached = db
        .lookup_score_by_hash(
            &cfg.database_url,
            &summary_hash,
            EvalMetric::HallucinationRate.as_str(),
            eval_version,
        )
        .await
        .map_err(ServiceError::InfraError)?;

    if let (Some(g_row), Some(h_row)) = (g_cached.as_ref(), h_cached.as_ref()) {
        outcomes.push(MetricOutcome {
            metric: g_row.metric.clone(),
            score: g_row.score,
            cached: true,
            judge_model: g_row.judge_model.clone(),
        });
        outcomes.push(MetricOutcome {
            metric: h_row.metric.clone(),
            score: h_row.score,
            cached: true,
            judge_model: h_row.judge_model.clone(),
        });
    } else {
        match judge_groundedness_for_summary(db, brainatlas, &cfg.database_url, &summary, &g_cfg).await {
            Ok(g_out) => {
                let details = Some(g_out.details);
                let judge_model = Some(g_out.judge_model);

                let g_res = services::score_with_cache(
                    db,
                    &cfg.database_url,
                    summary_id,
                    &summary_hash,
                    EvalMetric::ClaimGroundedness.as_str(),
                    eval_version,
                    || async {
                        Ok(ComputedScore {
                            score: g_out.claim_groundedness,
                            judge_model: judge_model.clone(),
                            details: details.clone(),
                        })
                    },
                )
                .await;
                let h_res = services::score_with_cache(
                    db,
                    &cfg.database_url,
                    summary_id,
                    &summary_hash,
                    EvalMetric::HallucinationRate.as_str(),
                    eval_version,
                    || async {
                        Ok(ComputedScore {
                            score: g_out.hallucination_rate,
                            judge_model: judge_model.clone(),
                            details: None,
                        })
                    },
                )
                .await;

                match g_res {
                    Ok(c) => outcomes.push(c.into()),
                    Err(e) => {
                        tracing::error!(error = %e, "claim_groundedness write failed");
                        if first_error.is_none() {
                            first_error = Some(format!("claim_groundedness: {e}"));
                        }
                    }
                }
                match h_res {
                    Ok(c) => outcomes.push(c.into()),
                    Err(e) => {
                        tracing::error!(error = %e, "hallucination_rate write failed");
                        if first_error.is_none() {
                            first_error = Some(format!("hallucination_rate: {e}"));
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "groundedness pipeline failed");
                if first_error.is_none() {
                    first_error = Some(format!("groundedness: {e}"));
                }
            }
        }
    }

    // -------- Rubric (5 metrics from one LLM call) --------
    let rubric_cache_keys = [
        EvalMetric::RubricRelevance,
        EvalMetric::RubricCoherence,
        EvalMetric::RubricSpecificity,
        EvalMetric::RubricClinicalUtility,
        EvalMetric::RubricTerminology,
    ];

    let mut rubric_cached: Vec<Option<domain::EvalScore>> = Vec::with_capacity(5);
    for m in rubric_cache_keys {
        let row = db
            .lookup_score_by_hash(&cfg.database_url, &summary_hash, m.as_str(), eval_version)
            .await
            .map_err(ServiceError::InfraError)?;
        rubric_cached.push(row);
    }

    if rubric_cached.iter().all(|r| r.is_some()) {
        for row in rubric_cached.into_iter().flatten() {
            outcomes.push(MetricOutcome {
                metric: row.metric,
                score: row.score,
                cached: true,
                judge_model: row.judge_model,
            });
        }
    } else {
        let r_cfg = RubricConfig {
            brainatlas_base_url: cfg.brainatlas_base_url.clone(),
            rubric_chat_model: cfg.rubric_chat_model.clone(),
        };
        match judge_rubric_for_summary(brainatlas, &summary, &r_cfg).await {
            Ok(entries) => {
                for entry in entries {
                    let score_val = entry.score;
                    let judge_model = Some(entry.judge_model.clone());
                    let details = Some(entry.details.clone());
                    let res = services::score_with_cache(
                        db,
                        &cfg.database_url,
                        summary_id,
                        &summary_hash,
                        entry.metric.as_str(),
                        eval_version,
                        || async {
                            Ok(ComputedScore {
                                score: score_val,
                                judge_model: judge_model.clone(),
                                details: details.clone(),
                            })
                        },
                    )
                    .await;
                    match res {
                        Ok(c) => outcomes.push(c.into()),
                        Err(e) => {
                            tracing::error!(metric = entry.metric.as_str(), error = %e, "rubric write failed");
                            if first_error.is_none() {
                                first_error = Some(format!("{}: {}", entry.metric.as_str(), e));
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "rubric pipeline failed");
                if first_error.is_none() {
                    first_error = Some(format!("rubric: {e}"));
                }
            }
        }
    }

    let final_status = if first_error.is_some() {
        EvalRunStatus::Failed
    } else {
        EvalRunStatus::Complete
    };
    let _ = db
        .upsert_run(
            &cfg.database_url,
            summary_id,
            eval_version,
            final_status,
            first_error.clone(),
        )
        .await;

    let _ = Utc::now();
    Ok(outcomes)
}
