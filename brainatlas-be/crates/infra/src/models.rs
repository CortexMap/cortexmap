use crate::schema::{
    brain_region_embeddings, llm_call_usage, llm_pricing, region_mapping, region_summary,
};
use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use diesel::prelude::*;
use domain::{BrainRegionEntry, RegionMapping, Rgb};
use uuid::Uuid;

/// Diesel row model for `region_mapping`. Private to `infra`.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = region_mapping)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RegionMappingRow {
    pub id: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub red: Option<i32>,
    pub green: Option<i32>,
    pub blue: Option<i32>,
    pub structure_order: Option<i32>,
    pub parent_region_id: Option<i32>,
    pub parent_acronym: Option<String>,
    pub created_at: Option<chrono::NaiveDateTime>,
}

impl From<RegionMappingRow> for RegionMapping {
    fn from(row: RegionMappingRow) -> Self {
        let color = match (row.red, row.green, row.blue) {
            (Some(r), Some(g), Some(b)) => Some(Rgb::new(
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
            )),
            _ => None,
        };
        let created_at: DateTime<Utc> = row
            .created_at
            .map(|ndt| Utc.from_utc_datetime(&ndt))
            .unwrap_or_else(Utc::now);

        RegionMapping {
            id: row.id,
            region_id: row.region_id,
            name: row.name,
            acronym: row.acronym,
            color,
            structure_order: row.structure_order,
            parent_region_id: row.parent_region_id,
            parent_acronym: row.parent_acronym,
            created_at,
        }
    }
}

/// Diesel row model for `region_summary`. Private to `infra`.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = region_summary)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RegionSummaryRow {
    pub id: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub summary: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub content_hash: Option<String>,
    pub is_active: bool,
    pub batch_id: Uuid,
}

impl From<RegionSummaryRow> for BrainRegionEntry {
    fn from(row: RegionSummaryRow) -> Self {
        let created_at = row
            .created_at
            .map(|ndt| Utc.from_utc_datetime(&ndt))
            .unwrap_or_else(Utc::now);

        BrainRegionEntry {
            region_id: row.region_id,
            name: row.name,
            acronym: row.acronym.unwrap_or_default(),
            summary: row.summary.unwrap_or_default(),
            created_at,
        }
    }
}

/// Diesel insert model for `region_summary`
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = region_summary)]
pub struct NewRegionSummaryRow {
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub summary: String,
    pub content_hash: Option<String>,
    pub batch_id: uuid::Uuid,
}

/// Diesel insert model for `brain_region_embeddings`
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = brain_region_embeddings)]
pub struct NewEmbeddingRow {
    pub region_id: i32,
    pub summary_id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub embedding: pgvector::Vector,
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: Option<String>,
    pub source_query: Option<String>,
    pub source_char_start: Option<i32>,
    pub source_char_end: Option<i32>,
}

/// Diesel queryable model for `brain_region_embeddings`
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = brain_region_embeddings)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EmbeddingRow {
    pub id: Uuid,
    pub region_id: i32,
    pub summary_id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub embedding: pgvector::Vector,
    pub created_at: NaiveDateTime,
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: Option<String>,
    pub source_query: Option<String>,
    pub source_char_start: Option<i32>,
    pub source_char_end: Option<i32>,
}

impl From<EmbeddingRow> for domain::SimilarChunk {
    fn from(row: EmbeddingRow) -> Self {
        domain::SimilarChunk {
            id: row.id,
            chunk_index: row.chunk_index,
            chunk_text: row.chunk_text,
            similarity_score: 0.0, // Set by query with distance calculation
            source_pmc_id: row.source_pmc_id,
            source_uid: row.source_uid,
            source_s3_key: row.source_s3_key,
            source_query: row.source_query,
            source_char_start: row.source_char_start,
            source_char_end: row.source_char_end,
        }
    }
}

/// Diesel row for the `llm_pricing` table.
///
/// All columns are declared to match the table shape so Diesel's
/// `Selectable`/`as_select()` can generate a type-checked SELECT. Not every
/// field is read by the application (e.g., `id`, `created_at`) but they must
/// exist on the struct for the Diesel macros to compile.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = llm_pricing)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[allow(dead_code)]
pub struct LlmPricingRow {
    pub id: Uuid,
    pub model: String,
    pub input_price_per_million: BigDecimal,
    pub output_price_per_million: BigDecimal,
    pub embedding_price_per_million: Option<BigDecimal>,
    pub currency: String,
    pub effective_from: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Diesel insert model for the `llm_call_usage` table.
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = llm_call_usage)]
pub struct NewLlmCallUsageRow {
    pub endpoint: String,
    pub model: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
    pub cost_usd: Option<BigDecimal>,
    pub correlation_id: Option<String>,
    pub region_id: Option<i32>,
    pub summary_id: Option<Uuid>,
    pub batch_id: Option<Uuid>,
    pub caller_tag: Option<String>,
    pub request_id: Option<String>,
}

/// Diesel queryable row for the `llm_call_usage` table.
///
/// All columns are declared to match the table shape so Diesel's
/// `Selectable`/`as_select()` can generate a type-checked SELECT. The
/// aggregation in `llm_usage::usage_aggregate` reads `model`, `cost_usd`,
/// `prompt_tokens`, `completion_tokens`, `total_tokens`, and `caller_tag`;
/// the remaining columns are kept on the struct to make the SELECT complete
/// and to preserve flexibility for future per-row queries.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = llm_call_usage)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[allow(dead_code)]
pub struct LlmCallUsageRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub endpoint: String,
    pub model: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
    pub cost_usd: Option<BigDecimal>,
    pub correlation_id: Option<String>,
    pub region_id: Option<i32>,
    pub summary_id: Option<Uuid>,
    pub batch_id: Option<Uuid>,
    pub caller_tag: Option<String>,
    pub request_id: Option<String>,
}
