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
