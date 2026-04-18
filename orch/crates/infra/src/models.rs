use chrono::NaiveDateTime;
use diesel::prelude::*;
use uuid::Uuid;

// Diesel models for database tables

#[derive(Queryable, Selectable, Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::processed_fetch_tasks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProcessedFetchTask {
    pub fetch_task_id: i64,
    pub region_id: Uuid,
    pub pmc_id: String,
    pub processed_at: NaiveDateTime,
    pub brainatlas_status: String,
    pub brainatlas_started_at: Option<NaiveDateTime>,
    pub brainatlas_completed_at: Option<NaiveDateTime>,
    pub error_message: Option<String>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::processed_fetch_tasks)]
pub struct NewProcessedFetchTask {
    pub fetch_task_id: i64,
    pub region_id: Uuid,
    pub pmc_id: String,
    pub brainatlas_status: String,
}

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::orch_config)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrchConfig {
    pub key: String,
    pub value: String,
    pub description: Option<String>,
    pub updated_at: NaiveDateTime,
}

#[derive(AsChangeset, Debug)]
#[diesel(table_name = crate::schema::orch_config)]
pub struct UpdateOrchConfig {
    pub value: String,
    pub updated_at: NaiveDateTime,
}

// Conversions from DB models to service models

impl From<ProcessedFetchTask> for services::ProcessedFetchTask {
    fn from(task: ProcessedFetchTask) -> Self {
        Self {
            fetch_task_id: task.fetch_task_id,
            region_id: task.region_id,
            processed_at: task.processed_at,
            brainatlas_status: task.brainatlas_status,
            brainatlas_started_at: task.brainatlas_started_at,
            brainatlas_completed_at: task.brainatlas_completed_at,
            error_message: task.error_message,
        }
    }
}

impl From<services::NewProcessedFetchTask> for NewProcessedFetchTask {
    fn from(task: services::NewProcessedFetchTask) -> Self {
        Self {
            fetch_task_id: task.fetch_task_id,
            region_id: task.region_id,
            pmc_id: String::new(), // Will be populated from fetcher API call
            brainatlas_status: task.brainatlas_status,
        }
    }
}

impl From<OrchConfig> for services::OrchConfig {
    fn from(config: OrchConfig) -> Self {
        Self {
            key: config.key,
            value: config.value,
            description: config.description,
            updated_at: config.updated_at,
        }
    }
}

// Batch processing models

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::region_queries)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RegionQueryRow {
    pub id: Uuid,
    pub query_text: String,
    pub source: String,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub region_id: Uuid,
}
#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::region_queries)]
pub struct NewRegionQuery {
    pub region_id: Uuid,
    pub query_text: String,
    pub source: String,
}

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::region_processing_batches)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProcessingBatchRow {
    pub id: Uuid,
    pub status: String,
    pub fetch_task_ids: Vec<Option<i64>>,
    pub expected_task_count: i32,
    pub content_hash: Option<String>,
    pub created_at: NaiveDateTime,
    pub ready_at: Option<NaiveDateTime>,
    pub processing_started_at: Option<NaiveDateTime>,
    pub completed_at: Option<NaiveDateTime>,
    pub summary_id: Option<Uuid>,
    pub error_message: Option<String>,
    pub region_id: Uuid, // Moved to end to match schema
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::region_processing_batches)]
pub struct NewProcessingBatch {
    pub region_id: Uuid,
    pub expected_task_count: i32,
}

// Conversions from DB models to domain models

impl From<RegionQueryRow> for domain::RegionQuery {
    fn from(row: RegionQueryRow) -> Self {
        use chrono::{DateTime, Utc};
        Self {
            id: row.id,
            region_id: row.region_id,
            query_text: row.query_text,
            source: domain::QuerySource::from(row.source.as_str()),
            priority: row.priority.unwrap_or(0),
            enabled: row.enabled.unwrap_or(true),
            created_at: DateTime::<Utc>::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::<Utc>::from_naive_utc_and_offset(row.updated_at, Utc),
        }
    }
}

impl From<ProcessingBatchRow> for domain::ProcessingBatch {
    fn from(row: ProcessingBatchRow) -> Self {
        use chrono::{DateTime, Utc};
        Self {
            id: row.id,
            region_id: row.region_id,
            status: domain::BatchStatus::from(row.status.as_str()),
            fetch_task_ids: row.fetch_task_ids.into_iter().flatten().collect(),
            expected_task_count: row.expected_task_count,
            content_hash: row.content_hash,
            created_at: DateTime::<Utc>::from_naive_utc_and_offset(row.created_at, Utc),
            ready_at: row
                .ready_at
                .map(|t| DateTime::<Utc>::from_naive_utc_and_offset(t, Utc)),
            processing_started_at: row
                .processing_started_at
                .map(|t| DateTime::<Utc>::from_naive_utc_and_offset(t, Utc)),
            completed_at: row
                .completed_at
                .map(|t| DateTime::<Utc>::from_naive_utc_and_offset(t, Utc)),
            summary_id: row.summary_id,
            error_message: row.error_message,
        }
    }
}

// Region Mapping Models

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::region_mapping)]
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
    pub created_at: Option<NaiveDateTime>,
}

impl From<RegionMappingRow> for services::RegionMapping {
    fn from(row: RegionMappingRow) -> Self {
        Self {
            id: row.id,
            region_id: row.region_id,
            name: row.name,
            acronym: row.acronym,
            red: row.red,
            green: row.green,
            blue: row.blue,
            structure_order: row.structure_order,
            parent_region_id: row.parent_region_id,
            parent_acronym: row.parent_acronym,
        }
    }
}

// Region Summary Models

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::region_summary)]
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

impl From<RegionSummaryRow> for services::RegionSummaryRecord {
    fn from(row: RegionSummaryRow) -> Self {
        use chrono::Utc;
        Self {
            id: row.id,
            summary: row.summary,
            created_at: row.created_at.unwrap_or_else(|| Utc::now().naive_utc()),
            batch_id: row.batch_id,
        }
    }
}

// Chunk Source Models (for source attribution)

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::brain_region_embeddings)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ChunkSourceRow {
    pub id: Uuid,
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_query: Option<String>,
}

impl From<ChunkSourceRow> for services::ChunkSourceRecord {
    fn from(row: ChunkSourceRow) -> Self {
        Self {
            id: row.id,
            source_pmc_id: row.source_pmc_id,
            source_uid: row.source_uid,
            source_query: row.source_query,
        }
    }
}

// Paper Metadata Models (for source attribution in process requests)

#[derive(QueryableByName, Debug, Clone)]
pub struct PaperMetadataRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub s3_key: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub pmc_id: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub uid: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub query: String,
}

// Reverse Search Models

#[derive(QueryableByName, Debug, Clone)]
pub struct SearchHitRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub region_uuid: Uuid,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub region_id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub name: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub acronym: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub summary_snippet: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub match_source: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub rank: f64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_count: i64,
}

impl From<SearchHitRow> for services::SearchHitRecord {
    fn from(row: SearchHitRow) -> Self {
        Self {
            region_uuid: row.region_uuid,
            region_id: row.region_id,
            name: row.name,
            acronym: row.acronym,
            summary_snippet: row.summary_snippet,
            match_source: row.match_source,
            rank: row.rank,
        }
    }
}
