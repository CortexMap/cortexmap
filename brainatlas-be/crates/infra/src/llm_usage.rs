//! Diesel-backed implementations of `LlmPricingRepo` and `LlmUsageRepo`.
//!
//! See `plans/2026-04-20-llm-cost-tracking-v1.md` (Phase 3/4).

use crate::InfraError;
use crate::models::{LlmCallUsageRow, LlmPricingRow, NewLlmCallUsageRow};
use crate::schema::llm_call_usage::dsl as usage_dsl;
use crate::schema::llm_pricing::dsl as price_dsl;
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive};
use chrono::{DateTime, Utc};
use deadpool_diesel::Runtime;
use deadpool_diesel::postgres::{BuildError, Manager, Pool};
use diesel::prelude::*;
use domain::{
    LlmPricing, NewLlmCallUsage, UsageAggregate, UsageAggregateFilter, UsageByCallerTag,
    UsageByModel,
};
use services::infra::{LlmPricingRepo, LlmUsageRepo};
use std::collections::HashMap;
use tokio::sync::OnceCell;

/// Shared connection pool for both repos. Lazily initialized by `pool()` on
/// the first call so we never build a pool until actually needed.
pub struct BrainAtlasLlmUsage {
    pool: OnceCell<Pool>,
}

impl BrainAtlasLlmUsage {
    pub fn new() -> Self {
        Self {
            pool: OnceCell::new(),
        }
    }

    async fn pool(&self, database_uri: &str) -> Result<&Pool, BuildError> {
        self.pool
            .get_or_try_init(|| async {
                let manager = Manager::new(database_uri, Runtime::Tokio1);
                Pool::builder(manager).max_size(4).build()
            })
            .await
    }
}

impl Default for BrainAtlasLlmUsage {
    fn default() -> Self {
        Self::new()
    }
}

fn row_to_pricing(row: LlmPricingRow) -> LlmPricing {
    LlmPricing {
        model: row.model,
        input_price_per_million: row
            .input_price_per_million
            .to_f64()
            .unwrap_or(0.0),
        output_price_per_million: row
            .output_price_per_million
            .to_f64()
            .unwrap_or(0.0),
        embedding_price_per_million: row
            .embedding_price_per_million
            .and_then(|d| d.to_f64()),
        currency: row.currency,
        effective_from: row.effective_from,
    }
}

fn new_usage_to_row(row: NewLlmCallUsage) -> NewLlmCallUsageRow {
    NewLlmCallUsageRow {
        endpoint: row.endpoint,
        model: row.model,
        prompt_tokens: row.prompt_tokens,
        completion_tokens: row.completion_tokens,
        total_tokens: row.total_tokens,
        cost_usd: row.cost_usd.and_then(BigDecimal::from_f64),
        correlation_id: row.correlation_id,
        region_id: row.region_id,
        summary_id: row.summary_id,
        batch_id: row.batch_id,
        caller_tag: row.caller_tag,
        request_id: row.request_id,
    }
}

#[async_trait::async_trait]
impl LlmPricingRepo for BrainAtlasLlmUsage {
    type Error = InfraError;

    async fn latest_for_model(
        &self,
        database_url: &str,
        model: &str,
    ) -> Result<Option<LlmPricing>, Self::Error> {
        let conn = self.pool(database_url).await?.get().await?;
        let model_owned = model.to_string();
        let row: Option<LlmPricingRow> = conn
            .interact(move |c| {
                price_dsl::llm_pricing
                    .filter(price_dsl::model.eq(model_owned))
                    .order(price_dsl::effective_from.desc())
                    .select(LlmPricingRow::as_select())
                    .first::<LlmPricingRow>(c)
                    .optional()
            })
            .await??;
        Ok(row.map(row_to_pricing))
    }
}

#[async_trait::async_trait]
impl LlmUsageRepo for BrainAtlasLlmUsage {
    type Error = InfraError;

    async fn record(
        &self,
        database_url: &str,
        row: NewLlmCallUsage,
    ) -> Result<(), Self::Error> {
        let conn = self.pool(database_url).await?.get().await?;
        let insert = new_usage_to_row(row);
        conn.interact(move |c| {
            diesel::insert_into(usage_dsl::llm_call_usage)
                .values(&insert)
                .execute(c)
        })
        .await??;
        Ok(())
    }

    async fn aggregate(
        &self,
        database_url: &str,
        filter: UsageAggregateFilter,
    ) -> Result<UsageAggregate, Self::Error> {
        let conn = self.pool(database_url).await?.get().await?;

        // Fetch matching rows and aggregate in-process. For the expected row
        // volumes this is simpler than crafting multi-GROUP BY diesel queries,
        // and the indexes on `(model, created_at)` / `(correlation_id)` keep
        // the row selection cheap.
        let rows: Vec<LlmCallUsageRow> = conn
            .interact(move |c| {
                let mut query = usage_dsl::llm_call_usage.into_boxed();
                if let Some(since) = filter.since {
                    query = query.filter(usage_dsl::created_at.ge(since));
                }
                if let Some(until) = filter.until {
                    query = query.filter(usage_dsl::created_at.le(until));
                }
                if let Some(m) = filter.model {
                    query = query.filter(usage_dsl::model.eq(m));
                }
                if let Some(cid) = filter.correlation_id {
                    query = query.filter(usage_dsl::correlation_id.eq(cid));
                }
                if let Some(prefix) = filter.correlation_id_prefix {
                    // Escape % and _ so they're treated as literals, then
                    // append `%` for prefix match.
                    let escaped = prefix.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
                    let like = format!("{}%", escaped);
                    query = query.filter(usage_dsl::correlation_id.like(like));
                }
                if let Some(rid) = filter.region_id {
                    query = query.filter(usage_dsl::region_id.eq(rid));
                }
                if let Some(sid) = filter.summary_id {
                    query = query.filter(usage_dsl::summary_id.eq(sid));
                }
                if let Some(bid) = filter.batch_id {
                    query = query.filter(usage_dsl::batch_id.eq(bid));
                }
                if let Some(tag) = filter.caller_tag {
                    query = query.filter(usage_dsl::caller_tag.eq(tag));
                }
                query
                    .select(LlmCallUsageRow::as_select())
                    .load::<LlmCallUsageRow>(c)
            })
            .await??;

        let mut total_cost = 0.0_f64;
        let mut total_tokens = 0_i64;
        let mut total_prompt = 0_i64;
        let mut total_completion = 0_i64;
        let total_calls = rows.len() as i64;

        let mut by_model: HashMap<String, (f64, i64, i64)> = HashMap::new(); // cost, tokens, calls
        let mut by_tag: HashMap<String, (f64, i64, i64)> = HashMap::new();

        for r in rows {
            let cost = r.cost_usd.as_ref().and_then(BigDecimal::to_f64).unwrap_or(0.0);
            total_cost += cost;
            total_tokens += r.total_tokens as i64;
            total_prompt += r.prompt_tokens as i64;
            total_completion += r.completion_tokens as i64;

            let entry = by_model.entry(r.model.clone()).or_insert((0.0, 0, 0));
            entry.0 += cost;
            entry.1 += r.total_tokens as i64;
            entry.2 += 1;

            if let Some(tag) = r.caller_tag.clone() {
                let te = by_tag.entry(tag).or_insert((0.0, 0, 0));
                te.0 += cost;
                te.1 += r.total_tokens as i64;
                te.2 += 1;
            }
        }

        let by_model_vec: Vec<UsageByModel> = by_model
            .into_iter()
            .map(|(model, (cost, tokens, calls))| UsageByModel {
                model,
                total_cost_usd: cost,
                total_tokens: tokens,
                total_calls: calls,
            })
            .collect();

        let by_tag_vec: Vec<UsageByCallerTag> = by_tag
            .into_iter()
            .map(|(tag, (cost, tokens, calls))| UsageByCallerTag {
                caller_tag: tag,
                total_cost_usd: cost,
                total_tokens: tokens,
                total_calls: calls,
            })
            .collect();

        Ok(UsageAggregate {
            total_cost_usd: total_cost,
            total_tokens,
            total_prompt_tokens: total_prompt,
            total_completion_tokens: total_completion,
            total_calls,
            by_model: by_model_vec,
            by_caller_tag: by_tag_vec,
        })
    }
}

// Silence "unused imports" warnings when clippy is run with no features that
// touch these paths.
#[allow(dead_code)]
fn _unused(d: DateTime<Utc>) -> DateTime<Utc> {
    d
}
