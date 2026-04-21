//! Cost guardrail background loop.
//!
//! Periodically polls brainatlas-be's `/api/llm/usage` for the rolling 24-hour
//! spend and emits `tracing::warn!` when the total exceeds a configured
//! threshold, `tracing::error!` when it breaches the hard daily budget.
//!
//! Reads two env vars:
//!   - `LLM_COST_DAILY_USD_BUDGET` — hard cap; error-level alerts above this
//!   - `LLM_COST_WARN_THRESHOLD_USD` — soft cap; warn-level alerts above this
//!
//! Neither env var triggers any enforcement (no calls are blocked). The point
//! is observability: an on-call engineer sees the error in logs and decides
//! whether to throttle manually via orch config or take brainatlas-be offline.

use crate::{EnvInfra, HttpClient, OrchDatabase, ServiceError};
use domain::ConfigKey;
use serde::Deserialize;
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
struct UsageAggregateWire {
    #[serde(default)]
    pub total_cost_usd: f64,
    #[serde(default)]
    pub total_calls: i64,
}

pub struct CostGuardrail<I> {
    infra: Arc<I>,
}

impl<I> CostGuardrail<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

impl<E, I> CostGuardrail<I>
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

    pub async fn is_enabled(&self) -> bool {
        self.get_config_string(ConfigKey::CostGuardrailEnabled)
            .await
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    pub async fn poll_interval_secs(&self) -> u64 {
        self.get_config_string(ConfigKey::CostGuardrailPollIntervalSecs)
            .await
            .and_then(|v| v.parse().ok())
            .unwrap_or(300)
    }

    async fn brainatlas_base_url(&self) -> Result<String, ServiceError<E>> {
        fn normalize_url(addr: &str) -> String {
            if addr.starts_with("http://") || addr.starts_with("https://") {
                addr.to_string()
            } else {
                let host_port = addr.replace("0.0.0.0", "localhost");
                format!("http://{}", host_port)
            }
        }
        if let Ok(url) = self.infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
            return Ok(normalize_url(&url));
        }
        let db_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;
        if let Some(url) = self
            .infra
            .get_config(&db_url, ConfigKey::BrainatlasBaseUrl)
            .await
            .map_err(ServiceError::InfraError)?
        {
            return Ok(normalize_url(&url));
        }
        Err(ServiceError::ConfigNotFound {
            key: "brainatlas_base_url".to_string(),
        })
    }

    /// Run one check. Returns the computed 24h spend on success, `None` when
    /// the brainatlas call fails (a transient network blip must not bring down
    /// the guardrail loop).
    pub async fn run_once(&self) -> Option<f64> {
        // Read thresholds fresh each cycle so an operator can tune them
        // without restarting orch.
        let daily_budget: Option<f64> = self
            .infra
            .get_env_var("LLM_COST_DAILY_USD_BUDGET")
            .ok()
            .and_then(|v| v.parse().ok());
        let warn_threshold: Option<f64> = self
            .infra
            .get_env_var("LLM_COST_WARN_THRESHOLD_USD")
            .ok()
            .and_then(|v| v.parse().ok());

        if daily_budget.is_none() && warn_threshold.is_none() {
            tracing::debug!("cost-guardrail: no thresholds configured; skipping check");
            return None;
        }

        let base = match self.brainatlas_base_url().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "cost-guardrail: brainatlas base url not resolvable");
                return None;
            }
        };

        let since = chrono::Utc::now() - chrono::Duration::hours(24);
        // RFC 3339 uses only `-`, `:`, `T`, `.`, and digits — all URL-safe
        // in a query string. No percent-encoding needed.
        let url = format!(
            "{}/brainatlas-be/api/llm/usage?since={}",
            base.trim_end_matches('/'),
            since.to_rfc3339()
        );

        let agg: UsageAggregateWire = match self.infra.get(&url).await {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(error = %e, url = %url, "cost-guardrail: usage fetch failed");
                return None;
            }
        };

        let correlation_id = Uuid::new_v4();
        if let Some(budget) = daily_budget
            && agg.total_cost_usd >= budget
        {
            tracing::error!(
                target: "llm.cost_guardrail",
                daily_budget_usd = budget,
                total_cost_usd = agg.total_cost_usd,
                total_calls = agg.total_calls,
                alert_id = %correlation_id,
                "cost-guardrail: 24h LLM spend EXCEEDS daily budget"
            );
        } else if let Some(warn) = warn_threshold
            && agg.total_cost_usd >= warn
        {
            tracing::warn!(
                target: "llm.cost_guardrail",
                warn_threshold_usd = warn,
                total_cost_usd = agg.total_cost_usd,
                total_calls = agg.total_calls,
                alert_id = %correlation_id,
                "cost-guardrail: 24h LLM spend exceeds warn threshold"
            );
        } else {
            tracing::info!(
                target: "llm.cost_guardrail",
                total_cost_usd = agg.total_cost_usd,
                total_calls = agg.total_calls,
                "cost-guardrail: 24h LLM spend within bounds"
            );
        }

        Some(agg.total_cost_usd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::{NewProcessedFetchTask, OrchConfig, ProcessedFetchTask};
    use async_trait::async_trait;
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tracing::{Event, Level, Subscriber};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    // ---- Error type for the fake infra ----

    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(String);

    // ---- Captured tracing event ----

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        level: Level,
        target: String,
        message: String,
        /// Concatenated `field=debug_value` pairs (excluding `message`).
        fields: String,
    }

    #[derive(Default, Clone)]
    struct CaptureStore {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl CaptureStore {
        fn new() -> Self {
            Self::default()
        }
        fn snapshot(&self) -> Vec<CapturedEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    /// Visitor that pulls every field out of an Event into a string.
    struct FieldVisitor {
        message: String,
        fields: String,
    }

    impl tracing::field::Visit for FieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = format!("{:?}", value);
                // Strip surrounding quotes from Debug of &str.
                if self.message.starts_with('"') && self.message.ends_with('"') {
                    self.message = self.message[1..self.message.len() - 1].to_string();
                }
            } else {
                if !self.fields.is_empty() {
                    self.fields.push(' ');
                }
                self.fields.push_str(&format!("{}={:?}", field.name(), value));
            }
        }
    }

    struct CaptureLayer {
        store: CaptureStore,
    }

    impl<S: Subscriber> Layer<S> for CaptureLayer {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut v = FieldVisitor {
                message: String::new(),
                fields: String::new(),
            };
            event.record(&mut v);
            let meta = event.metadata();
            self.store.events.lock().unwrap().push(CapturedEvent {
                level: *meta.level(),
                target: meta.target().to_string(),
                message: v.message,
                fields: v.fields,
            });
        }
    }

    /// Install a capture layer for the duration of the returned guard.
    /// Use `#[tokio::test(flavor = "current_thread")]` on the test so the
    /// thread-local dispatcher stays visible across every `.await`.
    fn install_capture() -> (CaptureStore, tracing::subscriber::DefaultGuard) {
        let store = CaptureStore::new();
        let subscriber = Registry::default().with(CaptureLayer {
            store: store.clone(),
        });
        let guard = tracing::subscriber::set_default(subscriber);
        (store, guard)
    }

    // ---- Mock infra ----

    type HttpResponder = Box<
        dyn Fn(&str) -> Result<serde_json::Value, MockErr> + Send + Sync,
    >;

    struct MockInfra {
        env: HashMap<String, String>,
        config: HashMap<String, String>,
        /// `None` means no response is staged; `Some(fn)` lets the test
        /// choose Ok/Err per URL.
        http_responder: Mutex<Option<HttpResponder>>,
    }

    impl MockInfra {
        fn new() -> Self {
            Self {
                env: HashMap::new(),
                config: HashMap::new(),
                http_responder: Mutex::new(None),
            }
        }
        fn with_env(mut self, k: &str, v: &str) -> Self {
            self.env.insert(k.to_string(), v.to_string());
            self
        }
        fn with_config(mut self, k: ConfigKey, v: &str) -> Self {
            self.config.insert(k.to_string(), v.to_string());
            self
        }
        fn with_http_ok(self, body: serde_json::Value) -> Self {
            *self.http_responder.lock().unwrap() =
                Some(Box::new(move |_url: &str| Ok(body.clone())));
            self
        }
        fn with_http_err(self) -> Self {
            *self.http_responder.lock().unwrap() = Some(Box::new(|_url: &str| {
                Err(MockErr("boom".to_string()))
            }));
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
            unimplemented!("not used by cost_guardrail tests")
        }
        async fn insert_processed_task(
            &self,
            _database_url: &str,
            _task: NewProcessedFetchTask,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by cost_guardrail tests")
        }
        async fn update_brainatlas_status(
            &self,
            _database_url: &str,
            _fetch_task_id: i64,
            _status: &str,
            _error: Option<String>,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by cost_guardrail tests")
        }
        async fn get_all_config(
            &self,
            _database_url: &str,
        ) -> Result<Vec<OrchConfig>, Self::Error> {
            unimplemented!("not used by cost_guardrail tests")
        }
        async fn update_config(
            &self,
            _database_url: &str,
            _key: ConfigKey,
            _value: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by cost_guardrail tests")
        }
    }

    #[async_trait]
    impl HttpClient for MockInfra {
        type Error = MockErr;

        async fn get<T: DeserializeOwned + Send>(
            &self,
            url: &str,
        ) -> Result<T, Self::Error> {
            let guard = self.http_responder.lock().unwrap();
            let responder = guard
                .as_ref()
                .expect("test did not stage an http responder");
            let value = responder(url)?;
            serde_json::from_value(value)
                .map_err(|e| MockErr(format!("deserialize: {}", e)))
        }

        async fn post<Req: Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
            &self,
            _url: &str,
            _body: &Req,
        ) -> Result<Res, Self::Error> {
            unimplemented!("not used by cost_guardrail tests")
        }

        async fn check_health(
            &self,
            _base_url: &str,
            _service_name: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!("not used by cost_guardrail tests")
        }
    }

    fn base_infra() -> MockInfra {
        MockInfra::new().with_env("DATABASE_URL", "postgres://mock")
    }

    fn usage_body(total_cost_usd: f64, total_calls: i64) -> serde_json::Value {
        serde_json::json!({
            "total_cost_usd": total_cost_usd,
            "total_calls": total_calls,
        })
    }

    // ---------- is_enabled ----------

    #[tokio::test]
    async fn is_enabled_true_when_config_set_true() {
        let infra = Arc::new(
            base_infra().with_config(ConfigKey::CostGuardrailEnabled, "true"),
        );
        let g = CostGuardrail::new(infra);
        assert!(g.is_enabled().await);
    }

    #[tokio::test]
    async fn is_enabled_true_ignores_case() {
        let infra = Arc::new(
            base_infra().with_config(ConfigKey::CostGuardrailEnabled, "TrUe"),
        );
        let g = CostGuardrail::new(infra);
        assert!(g.is_enabled().await);
    }

    #[tokio::test]
    async fn is_enabled_false_when_config_set_false() {
        let infra = Arc::new(
            base_infra().with_config(ConfigKey::CostGuardrailEnabled, "false"),
        );
        let g = CostGuardrail::new(infra);
        assert!(!g.is_enabled().await);
    }

    #[tokio::test]
    async fn is_enabled_false_when_config_missing() {
        let infra = Arc::new(base_infra());
        let g = CostGuardrail::new(infra);
        assert!(!g.is_enabled().await);
    }

    // ---------- poll_interval_secs ----------

    #[tokio::test]
    async fn poll_interval_defaults_to_300() {
        let infra = Arc::new(base_infra());
        let g = CostGuardrail::new(infra);
        assert_eq!(g.poll_interval_secs().await, 300);
    }

    #[tokio::test]
    async fn poll_interval_respects_config_override() {
        let infra = Arc::new(
            base_infra().with_config(ConfigKey::CostGuardrailPollIntervalSecs, "42"),
        );
        let g = CostGuardrail::new(infra);
        assert_eq!(g.poll_interval_secs().await, 42);
    }

    #[tokio::test]
    async fn poll_interval_defaults_when_config_unparseable() {
        let infra = Arc::new(
            base_infra().with_config(ConfigKey::CostGuardrailPollIntervalSecs, "not-a-num"),
        );
        let g = CostGuardrail::new(infra);
        assert_eq!(g.poll_interval_secs().await, 300);
    }

    // ---------- brainatlas_base_url ----------
    //
    // NOTE: the source reads env var `BRAINATLAS_HTTP_ADDR` (not
    // `BRAINATLAS_BASE_URL` as suggested in the prompt); the tests exercise
    // the real variable name used by the code under test.

    #[tokio::test]
    async fn brainatlas_base_url_from_env_var() {
        let infra = Arc::new(
            base_infra().with_env("BRAINATLAS_HTTP_ADDR", "http://env-host:9000"),
        );
        let g = CostGuardrail::new(infra);
        let url = g.brainatlas_base_url().await.expect("ok");
        assert_eq!(url, "http://env-host:9000");
    }

    #[tokio::test]
    async fn brainatlas_base_url_env_var_is_normalized() {
        // No scheme + 0.0.0.0 should be rewritten to http://localhost.
        let infra = Arc::new(
            base_infra().with_env("BRAINATLAS_HTTP_ADDR", "0.0.0.0:8082"),
        );
        let g = CostGuardrail::new(infra);
        let url = g.brainatlas_base_url().await.expect("ok");
        assert_eq!(url, "http://localhost:8082");
    }

    #[tokio::test]
    async fn brainatlas_base_url_from_config_when_env_unset() {
        let infra = Arc::new(
            base_infra().with_config(ConfigKey::BrainatlasBaseUrl, "http://cfg-host:8082"),
        );
        let g = CostGuardrail::new(infra);
        let url = g.brainatlas_base_url().await.expect("ok");
        assert_eq!(url, "http://cfg-host:8082");
    }

    #[tokio::test]
    async fn brainatlas_base_url_errors_when_both_unset() {
        let infra = Arc::new(base_infra());
        let g = CostGuardrail::new(infra);
        match g.brainatlas_base_url().await {
            Err(ServiceError::ConfigNotFound { key }) => {
                assert_eq!(key, "brainatlas_base_url");
            }
            other => panic!("expected ConfigNotFound, got {:?}", other.err()),
        }
    }

    #[tokio::test]
    async fn brainatlas_base_url_env_takes_precedence_over_config() {
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://env-wins:1")
                .with_config(ConfigKey::BrainatlasBaseUrl, "http://cfg-loses:2"),
        );
        let g = CostGuardrail::new(infra);
        let url = g.brainatlas_base_url().await.expect("ok");
        assert_eq!(url, "http://env-wins:1");
    }

    // ---------- run_once ----------

    #[tokio::test(flavor = "current_thread")]
    async fn run_once_short_circuits_when_no_thresholds() {
        let (store, _guard) = install_capture();
        let infra = Arc::new(
            base_infra().with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082"),
        );
        let g = CostGuardrail::new(infra);
        let result = g.run_once().await;
        let events = store.snapshot();
        assert_eq!(result, None);
        // No error/warn/info alert events on the `llm.cost_guardrail` target.
        for ev in &events {
            assert_ne!(
                ev.target, "llm.cost_guardrail",
                "unexpected alert event: {:?}",
                ev
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_once_returns_none_on_http_failure() {
        let (store, _guard) = install_capture();
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_env("LLM_COST_WARN_THRESHOLD_USD", "10")
                .with_http_err(),
        );
        let g = CostGuardrail::new(infra);
        let result = g.run_once().await;
        let events = store.snapshot();
        assert_eq!(result, None);
        // No alert event emitted on `llm.cost_guardrail` target; only a
        // generic warn about the fetch failure is acceptable.
        for ev in &events {
            assert_ne!(ev.target, "llm.cost_guardrail", "{:?}", ev);
        }
        // Confirm the fetch-failure warn was in fact emitted.
        assert!(
            events
                .iter()
                .any(|e| e.level == Level::WARN
                    && e.message.contains("usage fetch failed")),
            "expected a warn about fetch failure, got: {:#?}",
            events
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_once_error_when_cost_exceeds_daily_budget() {
        let (store, _guard) = install_capture();
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_env("LLM_COST_DAILY_USD_BUDGET", "50")
                .with_env("LLM_COST_WARN_THRESHOLD_USD", "10")
                .with_http_ok(usage_body(75.0, 123)),
        );
        let g = CostGuardrail::new(infra);
        let result = g.run_once().await;
        let events = store.snapshot();
        assert_eq!(result, Some(75.0));
        let alert = events
            .iter()
            .find(|e| e.target == "llm.cost_guardrail")
            .expect("expected alert event on llm.cost_guardrail target");
        assert_eq!(alert.level, Level::ERROR, "expected error-level alert");
        assert!(
            alert.message.contains("EXCEEDS daily budget"),
            "unexpected message: {}",
            alert.message
        );
        assert!(alert.fields.contains("daily_budget_usd=50"));
        assert!(alert.fields.contains("total_cost_usd=75"));
        assert!(alert.fields.contains("total_calls=123"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_once_warn_when_cost_between_warn_and_budget() {
        let (store, _guard) = install_capture();
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_env("LLM_COST_DAILY_USD_BUDGET", "50")
                .with_env("LLM_COST_WARN_THRESHOLD_USD", "10")
                .with_http_ok(usage_body(25.0, 7)),
        );
        let g = CostGuardrail::new(infra);
        let result = g.run_once().await;
        let events = store.snapshot();
        assert_eq!(result, Some(25.0));
        let alert = events
            .iter()
            .find(|e| e.target == "llm.cost_guardrail")
            .expect("expected alert event on llm.cost_guardrail target");
        assert_eq!(alert.level, Level::WARN, "expected warn-level alert");
        assert!(
            alert.message.contains("exceeds warn threshold"),
            "unexpected message: {}",
            alert.message
        );
        assert!(alert.fields.contains("warn_threshold_usd=10"));
        assert!(alert.fields.contains("total_cost_usd=25"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_once_info_when_cost_below_warn() {
        let (store, _guard) = install_capture();
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_env("LLM_COST_DAILY_USD_BUDGET", "50")
                .with_env("LLM_COST_WARN_THRESHOLD_USD", "10")
                .with_http_ok(usage_body(3.0, 2)),
        );
        let g = CostGuardrail::new(infra);
        let result = g.run_once().await;
        let events = store.snapshot();
        assert_eq!(result, Some(3.0));
        let alert = events
            .iter()
            .find(|e| e.target == "llm.cost_guardrail")
            .expect("expected alert event on llm.cost_guardrail target");
        assert_eq!(alert.level, Level::INFO, "expected info-level alert");
        assert!(
            alert.message.contains("within bounds"),
            "unexpected message: {}",
            alert.message
        );
        assert!(alert.fields.contains("total_cost_usd=3"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_once_warn_only_threshold_set_triggers_warn() {
        // daily_budget UNSET, only warn_threshold -> warn path when exceeded.
        let (store, _guard) = install_capture();
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_env("LLM_COST_WARN_THRESHOLD_USD", "10")
                .with_http_ok(usage_body(20.0, 1)),
        );
        let g = CostGuardrail::new(infra);
        let result = g.run_once().await;
        let events = store.snapshot();
        assert_eq!(result, Some(20.0));
        let alert = events
            .iter()
            .find(|e| e.target == "llm.cost_guardrail")
            .expect("alert");
        assert_eq!(alert.level, Level::WARN);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_once_budget_only_threshold_set_triggers_error() {
        // warn_threshold UNSET, only daily_budget -> error path when exceeded.
        let (store, _guard) = install_capture();
        let infra = Arc::new(
            base_infra()
                .with_env("BRAINATLAS_HTTP_ADDR", "http://b:8082")
                .with_env("LLM_COST_DAILY_USD_BUDGET", "50")
                .with_http_ok(usage_body(60.0, 9)),
        );
        let g = CostGuardrail::new(infra);
        let result = g.run_once().await;
        let events = store.snapshot();
        assert_eq!(result, Some(60.0));
        let alert = events
            .iter()
            .find(|e| e.target == "llm.cost_guardrail")
            .expect("alert");
        assert_eq!(alert.level, Level::ERROR);
    }
}
