//! Helpers used by the state-machine entry points in `app.rs`:
//! - compute + persist structural metrics
//! - probe cache for groundedness + rubric metrics
//!
//! The actual state machine lives in `services::state_machine`.

use crate::error::AppError;
use domain::EvalMetric;
use rpc_types::MetricResult;
use services::{
    structural, CachedScore, ComputedScore, EvalsDatabase, ServiceError, SummaryRow,
};
use std::error::Error;
use uuid::Uuid;

/// Convert a `CachedScore` into a wire `MetricResult`. Free function to avoid
/// orphan-rule issues (both types live in other crates).
pub fn cached_to_metric(c: CachedScore) -> MetricResult {
    MetricResult {
        metric: c.row.metric,
        score: c.row.score,
        cached: c.cached,
        judge_model: c.row.judge_model,
    }
}

/// Compute + cache the 4 structural metrics, appending `MetricResult` rows to
/// `out`. Returns the first failure (if any) but does not short-circuit: every
/// metric is attempted independently.
pub async fn run_structural_metrics<DB, E>(
    db: &DB,
    database_url: &str,
    summary_id: Uuid,
    summary_hash: &str,
    eval_version: &str,
    summary: &SummaryRow,
    out: &mut Vec<MetricResult>,
) -> Result<(), AppError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let summary_text = summary.summary.clone();
    let acronym = summary.acronym.clone();

    let mut first_err: Option<AppError<E>> = None;

    macro_rules! one {
        ($metric:expr, $compute:expr) => {{
            let res = services::score_with_cache(
                db,
                database_url,
                summary_id,
                summary_hash,
                $metric.as_str(),
                eval_version,
                || async { Ok(ComputedScore::structural($compute)) },
            )
            .await;
            match res {
                Ok(c) => out.push(cached_to_metric(c)),
                Err(e) => {
                    tracing::error!(metric = $metric.as_str(), error = %e, "structural metric failed");
                    if first_err.is_none() {
                        first_err = Some(AppError::from(e));
                    }
                }
            }
        }};
    }

    one!(
        EvalMetric::SectionCompleteness,
        structural::section_completeness(&summary_text)
    );
    one!(
        EvalMetric::LengthInRange,
        structural::length_in_range(&summary_text)
    );
    one!(
        EvalMetric::AcronymMention,
        structural::acronym_mention(&summary_text, acronym.as_deref())
    );
    one!(
        EvalMetric::NoPlaceholderText,
        structural::no_placeholder_text(&summary_text)
    );

    if let Some(e) = first_err {
        return Err(e);
    }
    Ok(())
}

/// Probe the `eval_scores` cache for the 2 groundedness metrics. Returns
/// `true` if BOTH are cached (and pushes them to `out`), `false` otherwise
/// (without pushing anything).
pub async fn probe_groundedness_cache<DB, E>(
    db: &DB,
    database_url: &str,
    summary_hash: &str,
    eval_version: &str,
    out: &mut Vec<MetricResult>,
) -> Result<bool, AppError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let g = db
        .lookup_score_by_hash(
            database_url,
            summary_hash,
            EvalMetric::ClaimGroundedness.as_str(),
            eval_version,
        )
        .await
        .map_err(ServiceError::InfraError)?;
    let h = db
        .lookup_score_by_hash(
            database_url,
            summary_hash,
            EvalMetric::HallucinationRate.as_str(),
            eval_version,
        )
        .await
        .map_err(ServiceError::InfraError)?;

    match (g, h) {
        (Some(g_row), Some(h_row)) => {
            out.push(MetricResult {
                metric: g_row.metric,
                score: g_row.score,
                cached: true,
                judge_model: g_row.judge_model,
            });
            out.push(MetricResult {
                metric: h_row.metric,
                score: h_row.score,
                cached: true,
                judge_model: h_row.judge_model,
            });
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Probe the `eval_scores` cache for all 5 rubric metrics. Returns `true` if
/// all 5 are cached (and pushes them to `out`), `false` otherwise.
pub async fn probe_rubric_cache<DB, E>(
    db: &DB,
    database_url: &str,
    summary_hash: &str,
    eval_version: &str,
    out: &mut Vec<MetricResult>,
) -> Result<bool, AppError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let keys = [
        EvalMetric::RubricRelevance,
        EvalMetric::RubricCoherence,
        EvalMetric::RubricSpecificity,
        EvalMetric::RubricClinicalUtility,
        EvalMetric::RubricTerminology,
    ];
    let mut rows = Vec::with_capacity(5);
    for m in keys {
        let row = db
            .lookup_score_by_hash(database_url, summary_hash, m.as_str(), eval_version)
            .await
            .map_err(ServiceError::InfraError)?;
        match row {
            Some(r) => rows.push(r),
            None => return Ok(false),
        }
    }
    for row in rows {
        out.push(MetricResult {
            metric: row.metric,
            score: row.score,
            cached: true,
            judge_model: row.judge_model,
        });
    }
    Ok(true)
}

/// Probe the `eval_scores` cache for any already-computed citation metrics
/// and push them to `out`. Unlike groundedness/rubric, this probe is
/// additive — it does not gate the state machine; citations always ride on
/// the same run as a fresh groundedness pass. Rows that are not cached yet
/// will simply be (re-)computed by the state machine and written via
/// `score_with_cache`.
pub async fn probe_citation_cache<DB, E>(
    db: &DB,
    database_url: &str,
    summary_hash: &str,
    eval_version: &str,
    out: &mut Vec<MetricResult>,
) -> Result<(), AppError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    for m in [
        EvalMetric::CitationPresence,
        EvalMetric::CitationValidity,
        EvalMetric::CitationScope,
        EvalMetric::CitationSupport,
    ] {
        let row = db
            .lookup_score_by_hash(database_url, summary_hash, m.as_str(), eval_version)
            .await
            .map_err(ServiceError::InfraError)?;
        if let Some(r) = row {
            out.push(MetricResult {
                metric: r.metric,
                score: r.score,
                cached: true,
                judge_model: r.judge_model,
            });
        }
    }
    Ok(())
}
