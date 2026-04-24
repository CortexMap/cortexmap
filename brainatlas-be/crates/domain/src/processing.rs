use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Retrieval fallback policy for `search_embeddings`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalFallbackPolicy {
    /// Search only within the current summary's embeddings.
    None,
    /// If current-summary retrieval is empty, retry against the region's
    /// currently active summary.
    ActiveSummary,
}

/// Retrieval scope passed from app → services → infra for vector search.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetrievalScope {
    pub region_id: i32,
    pub summary_id: Uuid,
    pub fallback_policy: RetrievalFallbackPolicy,
}

impl RetrievalScope {
    pub fn current_summary(region_id: i32, summary_id: Uuid) -> Self {
        Self {
            region_id,
            summary_id,
            fallback_policy: RetrievalFallbackPolicy::None,
        }
    }

    pub fn with_fallback_policy(mut self, fallback_policy: RetrievalFallbackPolicy) -> Self {
        self.fallback_policy = fallback_policy;
        self
    }
}

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
    // Character offsets within the source S3 file
    pub source_char_start: Option<i32>,
    pub source_char_end: Option<i32>,
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
    /// Unique identifier for this chunk (brain_region_embeddings PK)
    pub id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub similarity_score: f64,
    // Source metadata for citation
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_s3_key: Option<String>,
    pub source_query: Option<String>,
    pub source_char_start: Option<i32>,
    pub source_char_end: Option<i32>,
}

/// Full source details for a single chunk (returned by chunk source resolution endpoint)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSource {
    pub chunk_id: Uuid,
    pub chunk_text: String,
    pub source_s3_key: Option<String>,
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_query: Option<String>,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
}
