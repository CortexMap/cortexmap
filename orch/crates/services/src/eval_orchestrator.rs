//! Eval orchestrator — loop driver for the stateless evals-be service.
//!
//! As of 2026-04-19 evals-be makes ZERO outbound HTTP calls. All LLM calls
//! flow through orch:
//!
//!  1. orch calls `POST /evals-be/api/evals/score/init` with a summary id.
//!  2. evals-be returns a `NextAction::CallLlm { path, body, step_id }`
//!     describing the LLM request it wants run on its behalf.
//!  3. orch POSTs `body` to `{brainatlas_base_url}{path}` and gets the LLM
//!     response JSON.
//!  4. orch POSTs that response back to `POST /evals-be/api/evals/score/step`
//!     with the original `run_id` + `step_id` inside a typed
//!     `LlmResponsePayload`.
//!  5. orch loops on (3)+(4) until `NextAction::Done` arrives.
//!
//! GET-shaped endpoints (`/status`, `/worst`) are unchanged.

use crate::{EnvInfra, HttpClient, OrchDatabase, ServiceError};
use app::{EvalMetricStatsView, EvalStatusSummary, EvalWorstOffenderEntry, EvalWorstOffenders};
use domain::ConfigKey;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

// ---- Wire-type mirrors (matching evals-be/crates/rpc-types) ----

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UnscoredResponse {
    pub eval_version: String,
    pub limit: i64,
    pub summary_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InitScoreRequest {
    pub summary_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InitScoreResponse {
    pub run_id: Uuid,
    pub summary_id: Uuid,
    pub eval_version: String,
    pub next: NextAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StepRequest {
    pub run_id: Uuid,
    pub step_id: Uuid,
    pub llm_response: LlmResponsePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StepResponse {
    pub run_id: Uuid,
    pub next: NextAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NextAction {
    CallLlm {
        step_id: Uuid,
        endpoint: LlmEndpoint,
        path: String,
        body: serde_json::Value,
    },
    Done {
        #[serde(default)]
        metrics: Vec<MetricResult>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LlmEndpoint {
    ExtractClaims,
    Embed,
    JudgeGroundedness,
    JudgeRubric,
    JudgeCitation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LlmResponsePayload {
    Claims(serde_json::Value),
    Embed(serde_json::Value),
    Groundedness(serde_json::Value),
    Rubric(serde_json::Value),
    CitationSupport(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricResult {
    pub metric: String,
    pub score: f32,
    pub cached: bool,
    pub judge_model: Option<String>,
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
/// Safety bound on the number of CallLlm steps orch will execute for a
/// single summary before giving up.
const MAX_STEPS_PER_RUN: usize = 100;

/// Minimal mirror of brainatlas-be's `UsageAggregate` response. We only use
/// the scalar totals for the run-cost endpoint.
#[derive(Debug, Clone, Deserialize)]
struct UsageAggregateWire {
    #[serde(default)]
    pub total_cost_usd: f64,
    #[serde(default)]
    pub total_prompt_tokens: i64,
    #[serde(default)]
    pub total_completion_tokens: i64,
    #[serde(default)]
    pub total_calls: i64,
}

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
    async fn get_config_string(&self, key: ConfigKey) -> Option<String> {
        let database_url = self.infra.get_env_var("DATABASE_URL").ok()?;
        self.infra
            .get_config(&database_url, key)
            .await
            .ok()
            .flatten()
    }

    async fn evals_base_url(&self) -> Result<String, ServiceError<E>> {
        if let Ok(url) = self.infra.get_env_var("EVALS_BASE_URL") {
            return Ok(normalize_url(&url));
        }
        if let Some(url) = self.get_config_string(ConfigKey::EvalsBaseUrl).await {
            return Ok(normalize_url(&url));
        }
        Err(ServiceError::ConfigNotFound {
            key: "evals_base_url (EVALS_BASE_URL env or evals_base_url config row)".to_string(),
        })
    }

    /// Brainatlas base URL — used by orch to execute `NextAction::CallLlm`
    /// on evals-be's behalf. Prefer env (`BRAINATLAS_HTTP_ADDR`) then config.
    async fn brainatlas_base_url(&self) -> Result<String, ServiceError<E>> {
        if let Ok(url) = self.infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
            return Ok(normalize_url(&url));
        }
        if let Some(url) = self.get_config_string(ConfigKey::BrainatlasBaseUrl).await {
            return Ok(normalize_url(&url));
        }
        Err(ServiceError::ConfigNotFound {
            key: "brainatlas_base_url (BRAINATLAS_HTTP_ADDR env or brainatlas_base_url config row)"
                .to_string(),
        })
    }

    async fn eval_version(&self) -> String {
        self.get_config_string(ConfigKey::EvalVersion)
            .await
            .unwrap_or_else(|| "v0.2.0".to_string())
    }

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

    /// Aggregate the LLM cost incurred by a single eval run.
    ///
    /// Every LLM call made on behalf of `score/step` is tagged with
    /// `correlation_id = "eval:{run_id}:{step_id}"` (see `run_cycle` below),
    /// so aggregating by prefix `eval:{run_id}:` yields the total cost for
    /// that run across all of its steps.
    pub async fn get_run_cost(
        &self,
        run_id: Uuid,
    ) -> Result<domain::EvalRunCost, ServiceError<E>> {
        let base = self.brainatlas_base_url().await?;
        let url = format!(
            "{}/brainatlas-be/api/llm/usage?correlation_id_prefix=eval:{}:",
            base.trim_end_matches('/'),
            run_id
        );
        let wire: UsageAggregateWire = self
            .infra
            .get(&url)
            .await
            .map_err(ServiceError::InfraError)?;
        Ok(domain::EvalRunCost {
            run_id: run_id.to_string(),
            total_cost_usd: format!("{:.6}", wire.total_cost_usd),
            total_input_tokens: wire.total_prompt_tokens,
            total_output_tokens: wire.total_completion_tokens,
            total_calls: wire.total_calls,
        })
    }

    /// Run a single orchestrator cycle: discover unscored summaries and
    /// drive the init→step→...→Done loop for each with the configured
    /// concurrency. Returns (succeeded, failed).
    pub async fn run_cycle(&self) -> Result<(usize, usize), ServiceError<E>> {
        let evals_base = self.evals_base_url().await?;
        let brainatlas_base = self.brainatlas_base_url().await?;
        let version = self.eval_version().await;
        let concurrency = self.concurrency().await;

        let unscored_url = format!(
            "{}/evals-be/api/evals/unscored?eval_version={}&limit={}",
            evals_base, version, UNSCORED_PAGE_SIZE
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

        let infra = Arc::clone(&self.infra);
        let evals_base = Arc::new(evals_base);
        let brainatlas_base = Arc::new(brainatlas_base);
        let version = Arc::new(version);

        let results: Vec<Result<(), ()>> = stream::iter(unscored.summary_ids)
            .map(|summary_id| {
                let infra = Arc::clone(&infra);
                let evals_base = Arc::clone(&evals_base);
                let brainatlas_base = Arc::clone(&brainatlas_base);
                let version = Arc::clone(&version);
                async move {
                    match drive_one(&*infra, &evals_base, &brainatlas_base, summary_id, &version)
                        .await
                    {
                        Ok(_) => Ok(()),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                %summary_id,
                                "Eval orchestrator: scoring loop failed"
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

    pub async fn get_status(&self) -> Result<EvalStatusSummary, ServiceError<E>> {
        let base_url = self.evals_base_url().await?;
        let version = self.eval_version().await;
        let url = format!(
            "{}/evals-be/api/evals/summary?eval_version={}",
            base_url, version
        );
        let wire: EvalSummaryWire = self
            .infra
            .get(&url)
            .await
            .map_err(ServiceError::InfraError)?;
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
        let wire: WorstOffendersWire = self
            .infra
            .get(&url)
            .await
            .map_err(ServiceError::InfraError)?;
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

/// Drive the full init→step→...→Done loop for one summary. Makes LLM calls
/// against `brainatlas_base` on evals-be's behalf.
async fn drive_one<I, E>(
    infra: &I,
    evals_base: &str,
    brainatlas_base: &str,
    summary_id: Uuid,
    eval_version: &str,
) -> Result<usize, Box<dyn Error + Send + Sync>>
where
    E: Error + Send + Sync + 'static,
    I: HttpClient<Error = E> + Send + Sync,
{
    let init_url = format!("{}/evals-be/api/evals/score/init", evals_base);
    let step_url = format!("{}/evals-be/api/evals/score/step", evals_base);

    let init_req = InitScoreRequest {
        summary_id,
        eval_version: Some(eval_version.to_string()),
    };
    let init_resp: InitScoreResponse = infra
        .post(&init_url, &init_req)
        .await
        .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;

    let mut run_id = init_resp.run_id;
    let mut next = init_resp.next;
    let mut step_count = 0usize;

    for _ in 0..MAX_STEPS_PER_RUN {
        match next {
            NextAction::Done { metrics } => {
                tracing::debug!(
                    %summary_id,
                    metric_count = metrics.len(),
                    step_count,
                    "eval loop complete"
                );
                return Ok(step_count);
            }
            NextAction::CallLlm {
                step_id,
                endpoint,
                path,
                body,
            } => {
                step_count += 1;
                // Call brainatlas with the body evals-be handed us, annotating
                // it with a correlation id so brainatlas persists the cost
                // row under `eval:{run_id}:{step_id}`. See
                // `plans/2026-04-20-llm-cost-tracking-v1.md` task 14.
                let llm_url = format!("{}{}", brainatlas_base, path);
                let mut body_with_corr = body.clone();
                if let Some(obj) = body_with_corr.as_object_mut() {
                    obj.insert(
                        "correlation_id".to_string(),
                        serde_json::Value::String(format!("eval:{}:{}", run_id, step_id)),
                    );
                }
                let llm_resp_json: serde_json::Value = infra
                    .post::<serde_json::Value, serde_json::Value>(&llm_url, &body_with_corr)
                    .await
                    .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;

                // Wrap into the typed envelope evals-be expects.
                let payload = match endpoint {
                    LlmEndpoint::ExtractClaims => LlmResponsePayload::Claims(llm_resp_json),
                    LlmEndpoint::Embed => LlmResponsePayload::Embed(llm_resp_json),
                    LlmEndpoint::JudgeGroundedness => {
                        LlmResponsePayload::Groundedness(llm_resp_json)
                    }
                    LlmEndpoint::JudgeRubric => LlmResponsePayload::Rubric(llm_resp_json),
                    LlmEndpoint::JudgeCitation => {
                        LlmResponsePayload::CitationSupport(llm_resp_json)
                    }
                };

                let step_req = StepRequest {
                    run_id,
                    step_id,
                    llm_response: payload,
                };
                let step_resp: StepResponse = infra
                    .post(&step_url, &step_req)
                    .await
                    .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
                run_id = step_resp.run_id;
                next = step_resp.next;
            }
        }
    }
    Err(format!(
        "eval loop for {} exceeded {} steps without Done",
        summary_id, MAX_STEPS_PER_RUN
    )
    .into())
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
