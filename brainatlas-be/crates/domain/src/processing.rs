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
    // Source metadata for citation
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: Option<String>,
    pub source_query: Option<String>,
}

/// Summary to insert into database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRegionSummary {
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub summary: String,
    pub content_hash: String,
    pub batch_id: Uuid,
}

/// Existing summary found via content hash (deduplication)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingSummary {
    pub summary_id: Uuid,
    pub summary: String,
}

/// Result of processing a region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessResult {
    pub summary_id: Uuid,
    pub chunks_processed: usize,
    pub embeddings_created: usize,
    pub was_deduplicated: bool,
}

/// A chunk returned from vector similarity search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarChunk {
    pub chunk_index: i32,
    pub chunk_text: String,
    pub similarity_score: f64,
    // Source metadata for citation
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: Option<String>,
    pub source_query: Option<String>,
}
