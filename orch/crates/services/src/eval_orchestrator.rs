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
    pub async fn get_run_cost(&self, run_id: Uuid) -> Result<domain::EvalRunCost, ServiceError<E>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::{NewProcessedFetchTask, OrchConfig, ProcessedFetchTask};
    use async_trait::async_trait;
    use serde::de::DeserializeOwned;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ---- Error type ----
    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(String);

    // ---- HTTP responder by path ----
    //
    // Each staged responder returns a `Vec<serde_json::Value>` (or errors) to
    // emulate a sequence of responses for successive calls to the same path.
    // The router matches by path suffix: the longest-matching registered key
    // against the request URL wins.
    type Responder = Box<dyn Fn() -> Result<serde_json::Value, MockErr> + Send + Sync>;

    #[derive(Default)]
    struct Router {
        // ordered list (pattern, sequence-of-responders). We pop from index 0
        // each time `get`/`post` is called for the matching pattern. If the
        // sequence is exhausted we repeat the last entry.
        routes: Mutex<Vec<(String, Vec<Responder>, AtomicUsize)>>,
        calls: Mutex<Vec<(String, String, Option<serde_json::Value>)>>, // (method, url, body)
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        per_call_delay_ms: AtomicUsize,
    }

    impl Router {
        fn new() -> Self {
            Self::default()
        }
        fn route_ok(&self, pattern: &str, value: serde_json::Value) {
            let v = value.clone();
            self.route_seq(pattern, vec![Box::new(move || Ok(v.clone())) as Responder]);
        }
        fn route_seq(&self, pattern: &str, responders: Vec<Responder>) {
            self.routes.lock().unwrap().push((
                pattern.to_string(),
                responders,
                AtomicUsize::new(0),
            ));
        }
        fn set_delay_ms(&self, ms: usize) {
            self.per_call_delay_ms.store(ms, Ordering::SeqCst);
        }
        fn respond(
            &self,
            method: &str,
            url: &str,
            body: Option<serde_json::Value>,
        ) -> Result<serde_json::Value, MockErr> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), url.to_string(), body));

            let routes = self.routes.lock().unwrap();
            // Find the longest matching pattern the URL contains.
            let mut best: Option<usize> = None;
            let mut best_len = 0usize;
            for (i, (pat, _, _)) in routes.iter().enumerate() {
                if url.contains(pat.as_str()) && pat.len() > best_len {
                    best_len = pat.len();
                    best = Some(i);
                }
            }
            let idx = match best {
                Some(i) => i,
                None => {
                    return Err(MockErr(format!("no responder matches url: {}", url)));
                }
            };
            let (_, responders, cursor) = &routes[idx];
            let next = {
                let c = cursor.load(Ordering::SeqCst);
                if c + 1 < responders.len() {
                    cursor.fetch_add(1, Ordering::SeqCst);
                }
                c.min(responders.len() - 1)
            };
            responders[next]()
        }
    }

    // ---- MockInfra ----

    struct MockInfra {
        env: HashMap<String, String>,
        config: HashMap<String, String>,
        router: Router,
    }

    impl MockInfra {
        fn new() -> Self {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_string(), "postgres://mock".to_string());
            Self {
                env,
                config: HashMap::new(),
                router: Router::new(),
            }
        }
        fn with_env(mut self, k: &str, v: &str) -> Self {
            self.env.insert(k.to_string(), v.to_string());
            self
        }
        fn with_config(mut self, key: ConfigKey, v: &str) -> Self {
            self.config.insert(key.to_string(), v.to_string());
            self
        }
    }

    impl EnvInfra for MockInfra {
        type Error = MockErr;
        fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
            self.env
                .get(key)
                .cloned()
                .ok_or_else(|| MockErr(format!("no env {}", key)))
        }
    }

    #[async_trait]
    impl OrchDatabase for MockInfra {
        type Error = MockErr;

        async fn get_config(
            &self,
            _database_url: &str,
            key: ConfigKey,
        ) -> Result<Option<String>, Self::Error> {
            Ok(self.config.get(&key.to_string()).cloned())
        }

        async fn get_processed_task(
            &self,
            _database_url: &str,
            _fetch_task_id: i64,
        ) -> Result<Option<ProcessedFetchTask>, Self::Error> {
            unimplemented!()
        }
        async fn insert_processed_task(
            &self,
            _database_url: &str,
            _task: NewProcessedFetchTask,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn update_brainatlas_status(
            &self,
            _database_url: &str,
            _fetch_task_id: i64,
            _status: &str,
            _error: Option<String>,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn get_all_config(
            &self,
            _database_url: &str,
        ) -> Result<Vec<OrchConfig>, Self::Error> {
            unimplemented!()
        }
        async fn update_config(
            &self,
            _database_url: &str,
            _key: ConfigKey,
            _value: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl HttpClient for MockInfra {
        type Error = MockErr;

        async fn get<T: DeserializeOwned + Send>(&self, url: &str) -> Result<T, Self::Error> {
            self.router.in_flight.fetch_add(1, Ordering::SeqCst);
            let cur = self.router.in_flight.load(Ordering::SeqCst);
            self.router.max_in_flight.fetch_max(cur, Ordering::SeqCst);
            let delay = self.router.per_call_delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
            }
            let v = self.router.respond("GET", url, None);
            self.router.in_flight.fetch_sub(1, Ordering::SeqCst);
            let v = v?;
            serde_json::from_value(v).map_err(|e| MockErr(format!("deserialize: {}", e)))
        }

        async fn post<Req: serde::Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
            &self,
            url: &str,
            body: &Req,
        ) -> Result<Res, Self::Error> {
            self.router.in_flight.fetch_add(1, Ordering::SeqCst);
            let cur = self.router.in_flight.load(Ordering::SeqCst);
            self.router.max_in_flight.fetch_max(cur, Ordering::SeqCst);
            let delay = self.router.per_call_delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
            }
            let body_val = serde_json::to_value(body).ok();
            let v = self.router.respond("POST", url, body_val);
            self.router.in_flight.fetch_sub(1, Ordering::SeqCst);
            let v = v?;
            serde_json::from_value(v).map_err(|e| MockErr(format!("deserialize: {}", e)))
        }

        async fn check_health(
            &self,
            _base_url: &str,
            _service_name: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    // ---------- helpers ----------

    fn base_infra() -> MockInfra {
        MockInfra::new()
            .with_env("EVALS_BASE_URL", "http://evals:8083")
            .with_env("BRAINATLAS_HTTP_ADDR", "http://brain:8082")
    }

    fn init_resp(run_id: Uuid, next: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "run_id": run_id,
            "summary_id": Uuid::nil(),
            "eval_version": "v0.2.0",
            "next": next,
        })
    }

    fn call_llm(step_id: Uuid, endpoint: &str, path: &str) -> serde_json::Value {
        serde_json::json!({
            "kind": "call_llm",
            "step_id": step_id,
            "endpoint": endpoint,
            "path": path,
            "body": {"prompt": "hi"},
        })
    }

    fn done() -> serde_json::Value {
        serde_json::json!({ "kind": "done", "metrics": [] })
    }

    fn step_resp(run_id: Uuid, next: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "run_id": run_id,
            "next": next,
        })
    }

    fn unscored(ids: Vec<Uuid>) -> serde_json::Value {
        serde_json::json!({
            "eval_version": "v0.2.0",
            "limit": ids.len() as i64,
            "summary_ids": ids,
        })
    }

    // ---------- TEST 1: Full happy path ----------

    #[tokio::test]
    async fn run_cycle_happy_path_one_calllm_then_done() {
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let summary_id = Uuid::new_v4();

        let infra = Arc::new(base_infra());
        infra
            .router
            .route_ok("/evals-be/api/evals/unscored", unscored(vec![summary_id]));
        infra.router.route_ok(
            "/evals-be/api/evals/score/init",
            init_resp(
                run_id,
                call_llm(step_id, "extract_claims", "/brainatlas-be/api/llm/claims"),
            ),
        );
        infra.router.route_ok(
            "/brainatlas-be/api/llm/claims",
            serde_json::json!({ "claims": ["a"] }),
        );
        infra
            .router
            .route_ok("/evals-be/api/evals/score/step", step_resp(run_id, done()));

        let orch = EvalOrchestrator::new(infra.clone());
        let (ok, failed) = orch.run_cycle().await.expect("run_cycle ok");
        assert_eq!(ok, 1);
        assert_eq!(failed, 0);

        // Verify the correlation_id was injected into the LLM call body.
        let calls = infra.router.calls.lock().unwrap();
        let llm_post = calls
            .iter()
            .find(|(m, url, _)| m == "POST" && url.contains("/brainatlas-be/api/llm/claims"))
            .expect("llm call recorded");
        let body = llm_post.2.as_ref().expect("post body");
        let corr = body
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .expect("corr id");
        assert_eq!(corr, format!("eval:{}:{}", run_id, step_id));
    }

    // ---------- TEST 2: Every LlmEndpoint variant routes correctly ----------

    #[tokio::test]
    async fn every_llm_endpoint_variant_is_dispatched() {
        // Drive one run per endpoint, check the outbound step payload
        // wraps the LLM response in the matching LlmResponsePayload variant.
        let cases: Vec<(&str, &str, &str)> = vec![
            ("extract_claims", "/brainatlas-be/api/llm/claims", "claims"),
            ("embed", "/brainatlas-be/api/llm/embed", "embed"),
            (
                "judge_groundedness",
                "/brainatlas-be/api/llm/judge-ground",
                "groundedness",
            ),
            (
                "judge_rubric",
                "/brainatlas-be/api/llm/judge-rubric",
                "rubric",
            ),
            (
                "judge_citation",
                "/brainatlas-be/api/llm/judge-cite",
                "citation_support",
            ),
        ];

        for (endpoint, path, payload_kind) in cases {
            let run_id = Uuid::new_v4();
            let step_id = Uuid::new_v4();
            let summary_id = Uuid::new_v4();

            let infra = Arc::new(base_infra());
            infra
                .router
                .route_ok("/evals-be/api/evals/unscored", unscored(vec![summary_id]));
            infra.router.route_ok(
                "/evals-be/api/evals/score/init",
                init_resp(run_id, call_llm(step_id, endpoint, path)),
            );
            infra.router.route_ok(path, serde_json::json!({"ok": true}));
            infra
                .router
                .route_ok("/evals-be/api/evals/score/step", step_resp(run_id, done()));

            let orch = EvalOrchestrator::new(infra.clone());
            let (ok, failed) = orch.run_cycle().await.expect("run_cycle ok");
            assert_eq!(ok, 1, "endpoint {} should succeed", endpoint);
            assert_eq!(failed, 0);

            // Confirm the LLM URL was actually called on the brainatlas base.
            let calls = infra.router.calls.lock().unwrap();
            let posted = calls
                .iter()
                .any(|(m, url, _)| m == "POST" && url == &format!("http://brain:8082{}", path));
            assert!(
                posted,
                "expected POST to brainatlas base+path for {}",
                endpoint
            );

            // Confirm the step request wrapped the response under the right variant tag.
            let step_post = calls
                .iter()
                .find(|(m, url, _)| m == "POST" && url.contains("/score/step"))
                .expect("step call");
            let body = step_post.2.as_ref().expect("step body");
            let kind = body
                .get("llm_response")
                .and_then(|r| r.get("kind"))
                .and_then(|k| k.as_str())
                .expect("llm_response kind");
            assert_eq!(
                kind, payload_kind,
                "wrapping kind mismatch for {}",
                endpoint
            );
        }
    }

    // ---------- TEST 3: 5xx from brainatlas -> retry succeeds on 2nd call ----------
    //
    // Note: the current production code does NOT retry the brainatlas LLM
    // call inside `drive_one` — an error there propagates out of the loop
    // and the summary is marked `failed`. We exercise that behavior (single
    // 5xx kills the run) as the current contract. The plan's "retry on 2nd
    // attempt" assumption does not hold in source; we document and skip
    // the retry-success case, and instead assert fail-fast on a transient
    // 5xx matches the same output as sustained 5xx (the contract today).

    #[tokio::test]
    async fn transient_brainatlas_error_is_counted_as_failure() {
        // Current production code: one brainatlas 5xx fails the summary run.
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let summary_id = Uuid::new_v4();

        let infra = Arc::new(base_infra());
        infra
            .router
            .route_ok("/evals-be/api/evals/unscored", unscored(vec![summary_id]));
        infra.router.route_ok(
            "/evals-be/api/evals/score/init",
            init_resp(
                run_id,
                call_llm(step_id, "extract_claims", "/brainatlas-be/api/llm/claims"),
            ),
        );
        infra.router.route_seq(
            "/brainatlas-be/api/llm/claims",
            vec![
                Box::new(|| Err(MockErr("500".to_string()))) as Responder,
                Box::new(|| Ok(serde_json::json!({"claims": []}))) as Responder,
            ],
        );
        infra
            .router
            .route_ok("/evals-be/api/evals/score/step", step_resp(run_id, done()));

        let orch = EvalOrchestrator::new(infra.clone());
        let (ok, failed) = orch.run_cycle().await.expect("run_cycle ok");
        assert_eq!(ok, 0);
        assert_eq!(failed, 1);
    }

    // ---------- TEST 4: Sustained 5xx -> fail-fast classified ----------

    #[tokio::test]
    async fn sustained_5xx_fails_the_run() {
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let summary_id = Uuid::new_v4();

        let infra = Arc::new(base_infra());
        infra
            .router
            .route_ok("/evals-be/api/evals/unscored", unscored(vec![summary_id]));
        infra.router.route_ok(
            "/evals-be/api/evals/score/init",
            init_resp(
                run_id,
                call_llm(step_id, "embed", "/brainatlas-be/api/llm/embed"),
            ),
        );
        infra.router.route_seq(
            "/brainatlas-be/api/llm/embed",
            vec![
                Box::new(|| Err(MockErr("500-1".into()))) as Responder,
                Box::new(|| Err(MockErr("500-2".into()))) as Responder,
                Box::new(|| Err(MockErr("500-3".into()))) as Responder,
            ],
        );

        let orch = EvalOrchestrator::new(infra.clone());
        let (ok, failed) = orch.run_cycle().await.expect("run_cycle ok");
        assert_eq!(ok, 0);
        assert_eq!(failed, 1);
    }

    // ---------- TEST 5: concurrency cap respected ----------

    #[tokio::test]
    async fn run_cycle_respects_concurrency_cap() {
        // 5 summaries, configured concurrency of 2. Each brainatlas call sleeps
        // for 30ms so the overlap is observable through `router.max_in_flight`.
        let infra = Arc::new(base_infra().with_config(ConfigKey::EvalOrchestratorConcurrency, "2"));
        infra.router.set_delay_ms(30);

        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        infra
            .router
            .route_ok("/evals-be/api/evals/unscored", unscored(ids.clone()));

        // All runs share the same init/step routes (the URL is the same;
        // router reuses the last responder once exhausted).
        let init_body = init_resp(
            Uuid::new_v4(),
            call_llm(Uuid::new_v4(), "embed", "/brainatlas-be/api/llm/embed"),
        );
        infra
            .router
            .route_ok("/evals-be/api/evals/score/init", init_body);
        infra
            .router
            .route_ok("/brainatlas-be/api/llm/embed", serde_json::json!({}));
        infra.router.route_ok(
            "/evals-be/api/evals/score/step",
            step_resp(Uuid::new_v4(), done()),
        );

        let orch = EvalOrchestrator::new(infra.clone());
        let (ok, failed) = orch.run_cycle().await.expect("run_cycle ok");
        assert_eq!(ok + failed, 5);

        // Max observed in-flight should not exceed the concurrency cap (2).
        // Because each run performs 3 sequential HTTP calls serialized per
        // summary, the overlap across summaries is what we measure.
        let peak = infra.router.max_in_flight.load(Ordering::SeqCst);
        assert!(peak <= 2, "max in-flight {} exceeded cap 2", peak);
        // And we should see at least 2 overlapping calls (proves parallelism).
        assert!(peak >= 2, "expected parallelism, saw peak {}", peak);
    }

    // ---------- TEST 6: GET /status passthrough ----------

    #[tokio::test]
    async fn get_status_passthrough_decodes_summary() {
        let infra = Arc::new(base_infra());
        infra.router.route_ok(
            "/evals-be/api/evals/summary",
            serde_json::json!({
                "eval_version": "v1",
                "total_summaries": 10,
                "total_scored": 5,
                "per_metric": {
                    "groundedness": {"avg": 0.8, "min": 0.1, "max": 1.0, "count": 5}
                }
            }),
        );

        let orch = EvalOrchestrator::new(infra);
        let status = orch.get_status().await.expect("status");
        assert_eq!(status.eval_version, "v1");
        assert_eq!(status.total_summaries, 10);
        assert_eq!(status.total_scored, 5);
        let gm = status.per_metric.get("groundedness").expect("metric");
        assert!((gm.avg - 0.8).abs() < 1e-6);
        assert_eq!(gm.count, 5);
    }

    // ---------- TEST 7: GET /worst passthrough ----------

    #[tokio::test]
    async fn get_worst_passthrough_decodes_offenders() {
        let sid = Uuid::new_v4();
        let infra = Arc::new(base_infra());
        infra.router.route_ok(
            "/evals-be/api/evals/worst",
            serde_json::json!({
                "metric": "groundedness",
                "limit": 5,
                "entries": [{
                    "summary_id": sid,
                    "region_name": "hippocampus",
                    "metric": "groundedness",
                    "score": 0.2,
                    "eval_version": "v1",
                }]
            }),
        );

        let orch = EvalOrchestrator::new(infra);
        let worst = orch
            .get_worst("groundedness".to_string(), 5)
            .await
            .expect("worst");
        assert_eq!(worst.metric, "groundedness");
        assert_eq!(worst.limit, 5);
        assert_eq!(worst.entries.len(), 1);
        assert_eq!(worst.entries[0].summary_id, sid);
        assert_eq!(worst.entries[0].region_name.as_deref(), Some("hippocampus"));
    }

    // ---------- TEST 8: get_run_cost aggregates correlation prefix ----------

    #[tokio::test]
    async fn get_run_cost_builds_correlation_prefix_url() {
        let run_id = Uuid::new_v4();
        let infra = Arc::new(base_infra());
        infra.router.route_ok(
            "/brainatlas-be/api/llm/usage?correlation_id_prefix=eval:",
            serde_json::json!({
                "total_cost_usd": 1.23456789_f64,
                "total_prompt_tokens": 100,
                "total_completion_tokens": 50,
                "total_calls": 7,
            }),
        );

        let orch = EvalOrchestrator::new(infra.clone());
        let cost = orch.get_run_cost(run_id).await.expect("cost");
        assert_eq!(cost.run_id, run_id.to_string());
        // Format is {:.6} on f64 -> "1.234568"
        assert_eq!(cost.total_cost_usd, "1.234568");
        assert_eq!(cost.total_input_tokens, 100);
        assert_eq!(cost.total_output_tokens, 50);
        assert_eq!(cost.total_calls, 7);

        // URL actually requested contains the prefix `eval:{run_id}:`
        let calls = infra.router.calls.lock().unwrap();
        let url = &calls.iter().find(|(m, _, _)| m == "GET").unwrap().1;
        assert!(
            url.contains(&format!("correlation_id_prefix=eval:{}:", run_id)),
            "url={}",
            url
        );
    }

    // ---------- TEST 9: normalize_url ----------

    #[test]
    fn normalize_url_passthrough_and_rewrite() {
        assert_eq!(normalize_url("http://x:1"), "http://x:1");
        assert_eq!(normalize_url("https://x"), "https://x");
        assert_eq!(normalize_url("0.0.0.0:8082"), "http://localhost:8082");
        assert_eq!(normalize_url("host:9"), "http://host:9");
    }

    // ---------- TEST 10: empty unscored -> 0,0 ----------

    #[tokio::test]
    async fn run_cycle_empty_unscored_returns_zero() {
        let infra = Arc::new(base_infra());
        infra
            .router
            .route_ok("/evals-be/api/evals/unscored", unscored(vec![]));

        let orch = EvalOrchestrator::new(infra);
        let (ok, failed) = orch.run_cycle().await.expect("ok");
        assert_eq!(ok, 0);
        assert_eq!(failed, 0);
    }
}
