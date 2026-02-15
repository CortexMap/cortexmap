use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Embedding to insert into database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewEmbedding {
    pub region_id: i32,
    pub summary_id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub embedding: Vec<f32>,
}

/// Summary to insert into database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRegionSummary {
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub summary: String,
    pub content_hash: String,
}

/// Existing summary found via content hash (deduplication)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingSummary {
    pub id: Uuid,
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

/// Result of processing a region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessResult {
    pub summary_id: Uuid,
    pub chunks_processed: usize,
    pub embeddings_created: usize,
    pub was_deduplicated: bool,
}
