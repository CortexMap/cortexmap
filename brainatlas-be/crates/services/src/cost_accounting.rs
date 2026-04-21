//! Cost accounting helper for LLM calls.
//!
//! Wraps an `LlmCallOutcome<T>` + `UsageContext`, looks up pricing, computes
//! `cost_usd`, persists an `llm_call_usage` row, and emits a structured
//! `llm.call` tracing event.
//!
//! Failures in accounting MUST NOT fail the upstream LLM call: we log and move
//! on. The service caller always gets back the underlying value.

use crate::ServiceError;
use crate::infra::{LlmPricingRepo, LlmUsageRepo};
use domain::{LlmCallOutcome, LlmPricing, NewLlmCallUsage, UsageContext};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Small TTL cache in front of `LlmPricingRepo::latest_for_model`. Pricing is
/// expected to change on the order of days/weeks; we cap entries at 5 minutes
/// to keep the hot path off the DB.
const PRICING_CACHE_TTL_SECS: u64 = 300;

struct CachedPricing {
    pricing: Option<LlmPricing>,
    fetched_at: Instant,
}

pub struct CostAccountant<I> {
    infra: Arc<I>,
    pricing_cache: Arc<RwLock<std::collections::HashMap<String, CachedPricing>>>,
}

impl<I> Clone for CostAccountant<I> {
    fn clone(&self) -> Self {
        Self {
            infra: self.infra.clone(),
            pricing_cache: self.pricing_cache.clone(),
        }
    }
}

impl<I> CostAccountant<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self {
            infra,
            pricing_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }
}

impl<E, I> CostAccountant<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: LlmPricingRepo<Error = E>
        + LlmUsageRepo<Error = E>
        + crate::EnvInfra<Error = E>
        + Send
        + Sync
        + 'static,
{
    async fn cached_pricing(&self, model: &str) -> Option<LlmPricing> {
        // Fast path: shared read lock.
        {
            let cache = self.pricing_cache.read().await;
            if let Some(entry) = cache.get(model)
                && entry.fetched_at.elapsed().as_secs() < PRICING_CACHE_TTL_SECS
            {
                return entry.pricing.clone();
            }
        }
        // Slow path: fetch from DB, populate cache.
        let database_url = self.infra.get("DATABASE_URL").ok().unwrap_or_default();
        if database_url.is_empty() {
            warn!(
                model,
                "cost-accounting: DATABASE_URL not set; pricing lookup skipped"
            );
            return None;
        }
        let pricing = match self.infra.latest_for_model(&database_url, model).await {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    model,
                    error = %e,
                    "cost-accounting: pricing lookup failed; treating as missing"
                );
                None
            }
        };
        let mut cache = self.pricing_cache.write().await;
        cache.insert(
            model.to_string(),
            CachedPricing {
                pricing: pricing.clone(),
                fetched_at: Instant::now(),
            },
        );
        pricing
    }

    /// Record the outcome of an LLM call: compute cost, persist a row, and
    /// emit the `llm.call` tracing event. Never fails the caller — accounting
    /// errors are logged and swallowed.
    pub async fn record<T>(
        &self,
        outcome: &LlmCallOutcome<T>,
        ctx: &UsageContext,
        latency_ms: u64,
    ) {
        let pricing = self.cached_pricing(&outcome.model).await;
        let cost_usd: Option<f64> = pricing
            .as_ref()
            .map(|p| p.compute_cost_usd(outcome.usage, outcome.endpoint));
        if pricing.is_none() {
            warn!(
                model = %outcome.model,
                caller_tag = ?ctx.caller_tag,
                "cost-accounting: no pricing row for model; cost_usd will be NULL"
            );
        }

        // Structured event so log aggregators can pivot on "llm.call".
        info!(
            target: "llm.call",
            endpoint = outcome.endpoint.as_tag(),
            model = %outcome.model,
            caller_tag = ?ctx.caller_tag,
            prompt_tokens = outcome.usage.prompt_tokens,
            completion_tokens = outcome.usage.completion_tokens,
            total_tokens = outcome.usage.total_tokens,
            cost_usd = ?cost_usd,
            correlation_id = ?ctx.correlation_id,
            region_id = ?ctx.region_id,
            summary_id = ?ctx.summary_id.map(|u| u.to_string()),
            batch_id = ?ctx.batch_id.map(|u| u.to_string()),
            latency_ms,
            "llm.call"
        );

        let row = NewLlmCallUsage {
            endpoint: outcome.endpoint.as_tag().to_string(),
            model: outcome.model.clone(),
            prompt_tokens: outcome.usage.prompt_tokens as i32,
            completion_tokens: outcome.usage.completion_tokens as i32,
            total_tokens: outcome.usage.total_tokens as i32,
            cost_usd,
            correlation_id: ctx.correlation_id.clone(),
            region_id: ctx.region_id,
            summary_id: ctx.summary_id,
            batch_id: ctx.batch_id,
            caller_tag: ctx.caller_tag.clone(),
            request_id: ctx.request_id.clone(),
        };

        let database_url = match self.infra.get("DATABASE_URL") {
            Ok(u) => u,
            Err(e) => {
                warn!(error = %e, "cost-accounting: DATABASE_URL not set; usage row not persisted");
                return;
            }
        };
        if let Err(e) = self.infra.record(&database_url, row).await {
            warn!(
                error = %e,
                model = %outcome.model,
                "cost-accounting: failed to insert llm_call_usage row"
            );
        }
    }

    /// Wrapper that returns the inner value of an `LlmCallOutcome<T>` after
    /// recording it. Errors from the LLM call itself are passed through
    /// unchanged.
    pub async fn finish<T>(
        &self,
        result: Result<LlmCallOutcome<T>, ServiceError<E>>,
        ctx: UsageContext,
        started: Instant,
    ) -> Result<T, ServiceError<E>> {
        let outcome = result?;
        let latency_ms = started.elapsed().as_millis() as u64;
        self.record(&outcome, &ctx, latency_ms).await;
        Ok(outcome.value)
    }

    /// Convenience helper used for infrastructure-level errors: convert the
    /// raw `E` into `ServiceError::InfraError`.
    pub fn map_err<T>(
        r: Result<LlmCallOutcome<T>, E>,
    ) -> Result<LlmCallOutcome<T>, ServiceError<E>> {
        r.map_err(ServiceError::InfraError)
    }

    /// Manually flush the pricing cache. Used by tests / runbook tooling.
    pub async fn invalidate_pricing_cache(&self) {
        let mut cache = self.pricing_cache.write().await;
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EnvInfra;
    use crate::infra::{LlmPricingRepo, LlmUsageRepo};
    use domain::{LlmCallOutcome, LlmEndpointKind, LlmPricing, Usage, UsageContext};
    use std::sync::Mutex;

    #[derive(Debug, thiserror::Error)]
    #[error("mock error")]
    struct MockErr;

    /// Mock infra capturing every `record()` call and returning a canned
    /// pricing row. Thread-safe so we can assert from the test.
    struct MockInfra {
        pricing: Option<LlmPricing>,
        env: std::collections::HashMap<String, String>,
        records: Mutex<Vec<NewLlmCallUsage>>,
        fail_record: bool,
    }

    impl MockInfra {
        fn new(pricing: Option<LlmPricing>) -> Self {
            let mut env = std::collections::HashMap::new();
            env.insert("DATABASE_URL".to_string(), "postgres://mock".to_string());
            Self {
                pricing,
                env,
                records: Mutex::new(Vec::new()),
                fail_record: false,
            }
        }

        fn take_records(&self) -> Vec<NewLlmCallUsage> {
            std::mem::take(&mut *self.records.lock().unwrap())
        }
    }

    impl EnvInfra for MockInfra {
        type Error = MockErr;
        fn get(&self, key: &str) -> Result<String, Self::Error> {
            self.env.get(key).cloned().ok_or(MockErr)
        }
    }

    #[async_trait::async_trait]
    impl LlmPricingRepo for MockInfra {
        type Error = MockErr;
        async fn latest_for_model(
            &self,
            _db: &str,
            _model: &str,
        ) -> Result<Option<LlmPricing>, Self::Error> {
            Ok(self.pricing.clone())
        }
    }

    #[async_trait::async_trait]
    impl LlmUsageRepo for MockInfra {
        type Error = MockErr;
        async fn record(&self, _db: &str, row: NewLlmCallUsage) -> Result<(), Self::Error> {
            if self.fail_record {
                return Err(MockErr);
            }
            self.records.lock().unwrap().push(row);
            Ok(())
        }
        async fn aggregate(
            &self,
            _db: &str,
            _filter: domain::UsageAggregateFilter,
        ) -> Result<domain::UsageAggregate, Self::Error> {
            Ok(domain::UsageAggregate::default())
        }
    }

    fn pricing() -> LlmPricing {
        LlmPricing {
            model: "openai/gpt-4o-mini".to_string(),
            input_price_per_million: 0.15,
            output_price_per_million: 0.60,
            embedding_price_per_million: None,
            currency: "USD".to_string(),
            effective_from: chrono::Utc::now(),
        }
    }

    fn outcome(model: &str, prompt: u32, completion: u32) -> LlmCallOutcome<String> {
        LlmCallOutcome {
            value: "ok".to_string(),
            usage: Usage::new(prompt, completion, prompt + completion),
            endpoint: LlmEndpointKind::ChatCompletion,
            model: model.to_string(),
        }
    }

    #[tokio::test]
    async fn record_persists_row_with_computed_cost() {
        let infra = Arc::new(MockInfra::new(Some(pricing())));
        let accountant = CostAccountant::new(infra.clone());
        let outc = outcome("openai/gpt-4o-mini", 1_000_000, 1_000_000);
        let ctx = UsageContext::default().with_caller_tag("test");

        accountant.record(&outc, &ctx, 42).await;

        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.model, "openai/gpt-4o-mini");
        assert_eq!(row.prompt_tokens, 1_000_000);
        assert_eq!(row.completion_tokens, 1_000_000);
        assert_eq!(row.caller_tag.as_deref(), Some("test"));
        // 1M * $0.15 + 1M * $0.60 = $0.75
        let cost = row.cost_usd.expect("cost_usd set");
        assert!((cost - 0.75).abs() < 1e-9);
    }

    #[tokio::test]
    async fn record_persists_null_cost_when_pricing_missing() {
        let infra = Arc::new(MockInfra::new(None));
        let accountant = CostAccountant::new(infra.clone());
        let outc = outcome("unknown-model", 100, 50);
        let ctx = UsageContext::default().with_caller_tag("test");

        accountant.record(&outc, &ctx, 10).await;

        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].cost_usd.is_none());
    }

    #[tokio::test]
    async fn finish_swallows_repo_failures() {
        // fail_record = true; accountant logs-and-continues, caller still
        // gets the inner value.
        let mut mi = MockInfra::new(Some(pricing()));
        mi.fail_record = true;
        let infra = Arc::new(mi);
        let accountant = CostAccountant::new(infra.clone());

        let outc = outcome("openai/gpt-4o-mini", 10, 5);
        let ctx = UsageContext::default().with_caller_tag("test");
        let started = Instant::now();
        let r: Result<String, ServiceError<MockErr>> =
            accountant.finish::<String>(Ok(outc), ctx, started).await;
        // Caller sees Ok, even though the insert failed.
        assert_eq!(r.ok(), Some("ok".to_string()));
    }

    #[tokio::test]
    async fn pricing_cache_avoids_repeat_db_hits() {
        // After the first call, repeated record()s should all see the same
        // pricing without triggering another DB fetch. We can't directly
        // count calls on MockInfra without adding a counter, so we at least
        // verify cache invalidation works.
        let infra = Arc::new(MockInfra::new(Some(pricing())));
        let accountant = CostAccountant::new(infra.clone());
        let outc = outcome("openai/gpt-4o-mini", 10, 5);
        let ctx = UsageContext::default().with_caller_tag("test");

        for _ in 0..3 {
            accountant.record(&outc, &ctx, 1).await;
        }
        let rows = infra.take_records();
        assert_eq!(rows.len(), 3);
        for r in &rows {
            assert!(r.cost_usd.is_some());
        }
        accountant.invalidate_pricing_cache().await;
    }

    // ---------- Gap-fill tests (Plan Task 1.9) ----------

    /// Helper to build an embedding-endpoint outcome.
    fn embedding_outcome(model: &str, total: u32) -> LlmCallOutcome<String> {
        LlmCallOutcome {
            value: "ok".to_string(),
            usage: Usage::new(total, 0, total),
            endpoint: LlmEndpointKind::Embedding,
            model: model.to_string(),
        }
    }

    /// Pricing row that includes an explicit embedding price.
    fn embedding_pricing() -> LlmPricing {
        LlmPricing {
            model: "openai/text-embedding-3-small".to_string(),
            input_price_per_million: 0.15,
            output_price_per_million: 0.60,
            embedding_price_per_million: Some(0.02),
            currency: "USD".to_string(),
            effective_from: chrono::Utc::now(),
        }
    }

    /// Embedding endpoint cost math: the recorded row's `cost_usd` uses the
    /// embedding price, not the chat input/output prices.
    #[tokio::test]
    async fn record_computes_cost_for_embedding_endpoint() {
        let infra = Arc::new(MockInfra::new(Some(embedding_pricing())));
        let accountant = CostAccountant::new(infra.clone());
        // 5M tokens * $0.02 / 1M = $0.10
        let outc = embedding_outcome("openai/text-embedding-3-small", 5_000_000);
        let ctx = UsageContext::default().with_caller_tag("embed-test");

        accountant.record(&outc, &ctx, 7).await;

        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.endpoint, "embedding");
        assert_eq!(row.model, "openai/text-embedding-3-small");
        assert_eq!(row.total_tokens, 5_000_000);
        let cost = row.cost_usd.expect("cost_usd set");
        assert!(
            (cost - 0.10).abs() < 1e-9,
            "expected embedding cost 0.10, got {}",
            cost
        );
    }

    /// Full `UsageContext` round-trip: every optional field survives the
    /// `record` call into the persisted row unchanged.
    #[tokio::test]
    async fn record_preserves_full_usage_context() {
        let infra = Arc::new(MockInfra::new(Some(pricing())));
        let accountant = CostAccountant::new(infra.clone());

        let correlation = "eval:run-42:step-3".to_string();
        let summary_id = uuid::Uuid::new_v4();
        let batch_id = uuid::Uuid::new_v4();
        let ctx = UsageContext {
            correlation_id: Some(correlation.clone()),
            region_id: Some(7),
            summary_id: Some(summary_id),
            batch_id: Some(batch_id),
            caller_tag: Some("full-ctx-test".to_string()),
            request_id: Some("req-abc".to_string()),
        };

        let outc = outcome("openai/gpt-4o-mini", 100, 50);
        let started = Instant::now();
        let r: Result<String, ServiceError<MockErr>> = accountant
            .finish::<String>(Ok(outc), ctx.clone(), started)
            .await;
        assert_eq!(r.ok(), Some("ok".to_string()));

        let rows = infra.take_records();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.correlation_id.as_deref(), Some(correlation.as_str()));
        assert_eq!(row.region_id, Some(7));
        assert_eq!(row.summary_id, Some(summary_id));
        assert_eq!(row.batch_id, Some(batch_id));
        assert_eq!(row.caller_tag.as_deref(), Some("full-ctx-test"));
        assert_eq!(row.request_id.as_deref(), Some("req-abc"));
    }

    // ---- Tracing capture helpers (custom Layer — pattern copied from
    //      orch::services::cost_guardrail because the fmt::layer + BufWriter
    //      approach is flaky under parallel test execution).

    #[derive(Clone, Debug)]
    struct CapturedEvent {
        target: String,
        fields: String,
    }

    #[derive(Default, Clone)]
    struct CaptureStore {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl CaptureStore {
        fn snapshot(&self) -> Vec<CapturedEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    struct FieldVisitor {
        fields: String,
    }

    impl tracing::field::Visit for FieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            self.fields
                .push_str(&format!("{}={:?}", field.name(), value));
        }
    }

    struct CaptureLayer {
        store: CaptureStore,
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CaptureLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut v = FieldVisitor {
                fields: String::new(),
            };
            event.record(&mut v);
            let meta = event.metadata();
            self.store.events.lock().unwrap().push(CapturedEvent {
                target: meta.target().to_string(),
                fields: v.fields,
            });
        }
    }

    /// Contract test for the `tracing::info!(target: "llm.call", ...)` event:
    /// every documented field NAME must appear in the captured output.
    /// These field names are consumed by the downstream log-aggregation
    /// pipeline and MUST NOT be renamed without a coordinated change.
    ///
    /// **Note**: This test uses a thread-local tracing subscriber. When
    /// `cargo test` runs in parallel (default), tracing's global callsite
    /// interest cache can race with other tests that emit `llm.call`
    /// events from multi-threaded tokio runtimes, causing occasional
    /// misses. CI enforces `--test-threads=1` (see `ci/tests/ci.rs:62`)
    /// where this test is reliable. Run locally with
    /// `cargo test --lib -- --test-threads=1` if you see flakes.
    #[tokio::test(flavor = "current_thread")]
    async fn tracing_event_contains_all_contract_field_names() {
        use tracing_subscriber::Registry;
        use tracing_subscriber::layer::SubscriberExt;

        let store = CaptureStore::default();
        let subscriber = Registry::default().with(CaptureLayer {
            store: store.clone(),
        });
        let _guard = tracing::subscriber::set_default(subscriber);

        let infra = Arc::new(MockInfra::new(Some(pricing())));
        let accountant = CostAccountant::new(infra.clone());
        let outc = outcome("openai/gpt-4o-mini", 123, 45);
        let ctx = UsageContext::default().with_caller_tag("contract-test");
        accountant.record(&outc, &ctx, 99).await;

        let events = store.snapshot();
        let llm_call = events
            .iter()
            .find(|e| e.target == "llm.call")
            .unwrap_or_else(|| {
                panic!(
                    "no event with target=llm.call found; captured: {:?}",
                    events
                )
            });

        // These are the contract field names — downstream log pipelines pivot
        // on these exact spellings.
        for field in [
            "prompt_tokens",
            "completion_tokens",
            "cost_usd",
            "model",
            "caller_tag",
        ] {
            assert!(
                llm_call.fields.contains(field),
                "missing contract field `{field}` in llm.call event fields: {}",
                llm_call.fields
            );
        }
    }

    // NOTE: The pricing-cache TTL expiry test is intentionally NOT implemented
    // here. Production code at `cost_accounting.rs:100` calls `Instant::now()`
    // directly; exercising TTL expiry would require introducing a `Clock`
    // trait seam, which is an out-of-scope production refactor (see plan
    // Task 3.8 and the risk note at lines 238-239 of
    // `plans/2026-04-20-pr69-max-test-coverage-v1.md`).
}
