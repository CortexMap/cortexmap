//! Read-through `eval_scores` cache.
//!
//! Single entry-point for any code that wants to write a score row. The cache
//! is keyed by `(summary_hash, metric, eval_version)` — see migration
//! `2026-04-19-000001-create_eval_scores`.
//!
//! Centralising this here means a future metric impl cannot accidentally
//! bypass the cache: it simply doesn't have direct DB write access.

use crate::infra::EvalsDatabase;
use crate::ServiceError;
use domain::{EvalScore, NewEvalScore};
use std::error::Error;
use std::future::Future;
use uuid::Uuid;

/// Outcome of a `score_with_cache` call: the persisted row plus a flag telling
/// callers whether the score came from the cache (no compute) or was freshly
/// computed.
#[derive(Debug, Clone)]
pub struct CachedScore {
    pub row: EvalScore,
    pub cached: bool,
}

/// Read-through cache for a single `(summary_hash, metric, eval_version)`.
///
/// 1. SELECT on the unique cache index. On hit: return immediately, **no
///    `compute()` call**.
/// 2. On miss: invoke `compute()` to produce a `(score, judge_model, details)`
///    tuple, then INSERT ... ON CONFLICT DO NOTHING and re-select to resolve
///    concurrent writers to the same row.
pub async fn score_with_cache<DB, F, Fut, E>(
    db: &DB,
    database_url: &str,
    summary_id: Uuid,
    summary_hash: &str,
    metric: &str,
    eval_version: &str,
    compute: F,
) -> Result<CachedScore, ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ComputedScore, ServiceError<E>>>,
{
    if let Some(row) = db
        .lookup_score_by_hash(database_url, summary_hash, metric, eval_version)
        .await
        .map_err(ServiceError::InfraError)?
    {
        tracing::debug!(
            metric = metric,
            summary_hash = summary_hash,
            eval_version = eval_version,
            "metric=eval_cache_hit"
        );
        return Ok(CachedScore { row, cached: true });
    }

    let computed = compute().await?;

    let new = NewEvalScore {
        summary_id,
        summary_hash: summary_hash.to_string(),
        metric: metric.to_string(),
        score: computed.score,
        judge_model: computed.judge_model,
        details: computed.details,
        eval_version: eval_version.to_string(),
    };

    let row = db
        .insert_score(database_url, new)
        .await
        .map_err(ServiceError::InfraError)?;

    Ok(CachedScore { row, cached: false })
}

/// Result of the `compute` closure passed to `score_with_cache`.
#[derive(Debug, Clone)]
pub struct ComputedScore {
    pub score: f32,
    pub judge_model: Option<String>,
    pub details: Option<serde_json::Value>,
}

impl ComputedScore {
    pub fn structural(score: f32) -> Self {
        Self {
            score,
            judge_model: None,
            details: None,
        }
    }
}
