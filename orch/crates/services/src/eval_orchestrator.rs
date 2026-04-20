//! Phase-4 eval orchestrator service.
//!
//! Periodically discovers active summaries that have not yet been scored
//! against the current `eval_version`, then fans out
//! `POST /evals-be/api/evals/score` calls with a configurable concurrency.
//!
//! Decoupled by design: orch never touches `eval_scores` / `eval_runs`
//! directly. It asks evals-be which summaries are unscored
//! (`GET /evals-be/api/evals/unscored?eval_version=...&limit=...`) and lets
//! evals-be do all the DB work. If evals-be is down, every cycle becomes a
//! no-op until it recovers — no retry queue or cursor state is needed.

use crate::{EnvInfra, HttpClient, OrchDatabase, ServiceError};
use app::{
    EvalMetricStatsView, EvalStatusSummary, EvalWorstOffenderEntry, EvalWorstOffenders,
};
use domain::ConfigKey;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

/// Wire types matching `evals-be/crates/rpc-types/src/lib.rs`. Duplicated here
/// to avoid a workspace-cross-dependency from orch into the evals-be crate
/// tree (orch and evals-be ship and version independently).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UnscoredResponse {
    pub eval_version: String,
    pub limit: i64,
    pub summary_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScoreRequest {
    pub summary_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricResult {
    pub metric: String,
    pub score: f32,
    pub cached: bool,
    pub judge_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScoreResponse {
    pub summary_id: Uuid,
    pub eval_version: String,
    pub metrics: Vec<MetricResult>,
}

/// JSON shape returned by `GET /evals-be/api/evals/summary`. Decoded into
/// `app::EvalStatusSummary` for the orch trait return type.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalSummaryWire {
    pub eval_version: String,
    pub total_summaries: i64,
    pub total_scored: i64,
    #[serde(default)]
    pub per_metric: std::collections::HashMap<String, MetricStatsWire>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricStatsWire {
    pub avg: f32,
    pub min: f32,
    pub max: f32,
    pub count: i64,
}

/// Wire shape for `GET /evals-be/api/evals/worst`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorstOffendersWire {
    pub metric: String,
    pub limit: i64,
    pub entries: Vec<WorstOffenderWire>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorstOffenderWire {
    pub summary_id: Uuid,
    pub region_name: Option<String>,
    pub metric: String,
    pub score: f32,
    pub eval_version: String,
}

/// How many summary IDs to request per `/unscored` call. Each cycle drains
/// up to this many; if there are more, the next cycle picks them up.
const UNSCORED_PAGE_SIZE: i64 = 200;

pub struct EvalOrchestrator<I> {
    infra: Arc<I>,
}

impl<I> EvalOrchestrator<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

impl<E, I> EvalOrchestrator<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + OrchDatabase<Error = E> + HttpClient<Error = E> + Send + Sync,
{
    /// Read the orch config table. Falls back to the `ConfigKey` default if
    /// the row is missing or unparsable.
    async fn get_config_string(&self, key: ConfigKey) -> Option<String> {
        let database_url = self.infra.get_env_var("DATABASE_URL").ok()?;
        self.infra
            .get_config(&database_url, key)
            .await
            .ok()
            .flatten()
    }

    async fn evals_base_url(&self) -> Result<String, ServiceError<E>> {
        // 1) explicit env var (preferred — survives DB outages)
        if let Ok(url) = self.infra.get_env_var("EVALS_BASE_URL") {
            return Ok(normalize_url(&url));
        }
        // 2) orch_config row
        if let Some(url) = self.get_config_string(ConfigKey::EvalsBaseUrl).await {
            return Ok(normalize_url(&url));
        }
        Err(ServiceError::ConfigNotFound {
            key: "evals_base_url (EVALS_BASE_URL env or evals_base_url config row)".to_string(),
        })
    }

    async fn eval_version(&self) -> String {
        self.get_config_string(ConfigKey::EvalVersion)
            .await
            .unwrap_or_else(|| "v1.0".to_string())
    }

    /// Returns true if the orchestrator is enabled in config.
    pub async fn is_enabled(&self) -> bool {
        self.get_config_string(ConfigKey::EvalOrchestratorEnabled)
            .await
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    pub async fn poll_interval_secs(&self) -> u64 {
        self.get_config_string(ConfigKey::EvalOrchestratorPollIntervalSecs)
            .await
            .and_then(|v| v.parse().ok())
            .unwrap_or(60)
    }

    pub async fn concurrency(&self) -> usize {
        self.get_config_string(ConfigKey::EvalOrchestratorConcurrency)
            .await
            .and_then(|v| v.parse().ok())
            .unwrap_or(5)
    }

    /// Run a single orchestrator cycle: discover unscored summaries and fan
    /// out score requests with the configured concurrency. Returns the count
    /// of (succeeded, failed) score invocations so the caller can log it.
    pub async fn run_cycle(&self) -> Result<(usize, usize), ServiceError<E>> {
        let base_url = self.evals_base_url().await?;
        let version = self.eval_version().await;
        let concurrency = self.concurrency().await;

        let unscored_url = format!(
            "{}/evals-be/api/evals/unscored?eval_version={}&limit={}",
            base_url, version, UNSCORED_PAGE_SIZE
        );

        let unscored: UnscoredResponse = self
            .infra
            .get(&unscored_url)
            .await
            .map_err(ServiceError::InfraError)?;

        if unscored.summary_ids.is_empty() {
            return Ok((0, 0));
        }

        tracing::info!(
            count = unscored.summary_ids.len(),
            eval_version = %version,
            "Eval orchestrator: discovered unscored summaries"
        );

        let score_url = format!("{}/evals-be/api/evals/score", base_url);
        let infra = Arc::clone(&self.infra);

        let results: Vec<Result<ScoreResponse, ()>> = stream::iter(unscored.summary_ids)
            .map(|summary_id| {
                let url = score_url.clone();
                let ver = version.clone();
                let infra = Arc::clone(&infra);
                async move {
                    let req = ScoreRequest {
                        summary_id,
                        eval_version: Some(ver),
                    };
                    match infra.post::<ScoreRequest, ScoreResponse>(&url, &req).await {
                        Ok(resp) => Ok(resp),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                %summary_id,
                                "Eval orchestrator: score request failed"
                            );
                            Err(())
                        }
                    }
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

        let succeeded = results.iter().filter(|r| r.is_ok()).count();
        let failed = results.len() - succeeded;
        Ok((succeeded, failed))
    }

    /// Fetch the per-metric aggregate from evals-be for the
    /// `/orch/api/evals/status` endpoint.
    pub async fn get_status(&self) -> Result<EvalStatusSummary, ServiceError<E>> {
        let base_url = self.evals_base_url().await?;
        let version = self.eval_version().await;
        let url = format!(
            "{}/evals-be/api/evals/summary?eval_version={}",
            base_url, version
        );
        let wire: EvalSummaryWire =
            self.infra.get(&url).await.map_err(ServiceError::InfraError)?;
        Ok(EvalStatusSummary {
            eval_version: wire.eval_version,
            total_summaries: wire.total_summaries,
            total_scored: wire.total_scored,
            per_metric: wire
                .per_metric
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        EvalMetricStatsView {
                            avg: v.avg,
                            min: v.min,
                            max: v.max,
                            count: v.count,
                        },
                    )
                })
                .collect(),
        })
    }

    /// Fetch the N worst-scoring summaries for the given metric. Backs
    /// `/orch/api/evals/worst` and the dashboard "Worst Offenders" table.
    pub async fn get_worst(
        &self,
        metric: String,
        limit: i64,
    ) -> Result<EvalWorstOffenders, ServiceError<E>> {
        let base_url = self.evals_base_url().await?;
        let version = self.eval_version().await;
        let url = format!(
            "{}/evals-be/api/evals/worst?metric={}&limit={}&eval_version={}",
            base_url, metric, limit, version
        );
        let wire: WorstOffendersWire =
            self.infra.get(&url).await.map_err(ServiceError::InfraError)?;
        Ok(EvalWorstOffenders {
            metric: wire.metric,
            limit: wire.limit,
            entries: wire
                .entries
                .into_iter()
                .map(|e| EvalWorstOffenderEntry {
                    summary_id: e.summary_id,
                    region_name: e.region_name,
                    metric: e.metric,
                    score: e.score,
                    eval_version: e.eval_version,
                })
                .collect(),
        })
    }
}

/// Normalize an HTTP address: prepend `http://` if missing, and replace
/// `0.0.0.0` host with `localhost` so loopback callers connect successfully.
fn normalize_url(addr: &str) -> String {
    if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        let host_port = addr.replace("0.0.0.0", "localhost");
        format!("http://{}", host_port)
    }
}
