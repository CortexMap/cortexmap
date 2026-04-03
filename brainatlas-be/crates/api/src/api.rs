use domain::rpc_types::{BrainRegionListResponse, ProcessRegionResponse, SearchBrainRegionResponse, StatusResponse, GenerateQueriesResponse, PaperMetadata};
use domain::ChunkSource;
use uuid::Uuid;

/// Response from the /api/summarize endpoint
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SummarizeResponse {
    pub summary_id: String,
    pub summary_text: String,
    pub region_name: String,
}

/// Request body for the /api/summarize endpoint
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SummarizeRequest {
    pub region_id: Option<domain::rpc_types::Uuid>,
    pub chat_model: Option<String>,
    pub embedding_model: Option<String>,
}

/// Request body for the /api/ingest endpoint (same as ProcessRegionRequest but no chat_model needed)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IngestRequest {
    pub region_id: Option<domain::rpc_types::Uuid>,
    pub batch_id: Option<domain::rpc_types::Uuid>,
    pub s3_keys: Vec<String>,
    #[serde(default)]
    pub paper_metadata: Vec<PaperMetadata>,
    pub embedding_model: Option<String>,
}

/// Response from the /api/ingest endpoint
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IngestResponse {
    pub region_id: Option<domain::rpc_types::Uuid>,
    pub detail: String,
}

#[async_trait::async_trait]
pub trait BrainRegionApi: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Search for all brain region summaries by UUID.
    /// Returns proto response ready to serialise.
    /// POST /api/search
    async fn search_brain_region(&self, id: Option<Uuid>) -> Result<SearchBrainRegionResponse, Self::Error>;

    /// List all brain region mappings.
    /// Returns proto response ready to serialise.
    /// GET /api/list
    async fn list_brain_regions(&self) -> Result<BrainRegionListResponse, Self::Error>;

    /// Get the processing status of a specific brain region by UUID.
    /// POST /api/status
    async fn status(&self, id: Uuid) -> Result<StatusResponse, Self::Error>;

    /// Chunk, embed, and summarize the S3 files for a region, then persist to region_summary.
    /// POST /api/process — called by orch (legacy)
    async fn process_region(
        &self,
        region_id: Option<Uuid>,
        batch_id: Option<Uuid>,
        s3_keys: Vec<String>,
        paper_metadata: Vec<PaperMetadata>,
        chat_model: Option<String>,
        embedding_model: Option<String>,
    ) -> Result<ProcessRegionResponse, Self::Error>;

    /// Chunk and embed S3 files without generating a summary.
    /// POST /api/ingest — called by orch completion watcher for periodic ingestion
    async fn ingest_region(
        &self,
        region_id: Option<Uuid>,
        batch_id: Option<Uuid>,
        s3_keys: Vec<String>,
        paper_metadata: Vec<PaperMetadata>,
        embedding_model: Option<String>,
    ) -> Result<IngestResponse, Self::Error>;

    /// Generate a summary using only existing embeddings (RAG-only, no fetching).
    /// POST /api/summarize — called by orch for on-demand summary generation
    async fn summarize_region(
        &self,
        region_id: Option<Uuid>,
        chat_model: Option<String>,
        embedding_model: Option<String>,
    ) -> Result<SummarizeResponse, Self::Error>;

    /// Generate search queries for a brain region using LLM.
    /// POST /api/generate-queries — called by orch when creating a new batch
    async fn generate_queries(&self, region_name: String, count: u32) -> Result<GenerateQueriesResponse, Self::Error>;

    /// Resolve a chunk UUID to its full source details.
    /// GET /api/chunks/{chunk_id}/source
    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<Option<ChunkSource>, Self::Error>;
}
