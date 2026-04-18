use domain::ChunkSource;
use domain::rpc_types::{
    BrainRegionListResponse, GenerateQueriesResponse, PaperMetadata, ProcessRegionResponse,
    SearchBrainRegionResponse, StatusResponse,
};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait BrainRegionApi: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Search for all brain region summaries by UUID.
    /// Returns proto response ready to serialise.
    /// POST /api/search
    async fn search_brain_region(
        &self,
        id: Option<Uuid>,
    ) -> Result<SearchBrainRegionResponse, Self::Error>;

    /// List all brain region mappings.
    /// Returns proto response ready to serialise.
    /// GET /api/list
    async fn list_brain_regions(&self) -> Result<BrainRegionListResponse, Self::Error>;

    /// Get the processing status of a specific brain region by UUID.
    /// POST /api/status
    async fn status(&self, id: Uuid) -> Result<StatusResponse, Self::Error>;

    /// Chunk, embed, and optionally summarize the S3 files for a region, then persist to region_summary.
    /// When skip_summarization is true, only chunks and embeds (no RAG summary).
    /// POST /api/process — called by orch
    async fn process_region(
        &self,
        region_id: Option<Uuid>,
        batch_id: Option<Uuid>,
        s3_keys: Vec<String>,
        paper_metadata: Vec<PaperMetadata>,
        chat_model: Option<String>,
        embedding_model: Option<String>,
        skip_summarization: bool,
    ) -> Result<ProcessRegionResponse, Self::Error>;

    /// Generate search queries for a brain region using LLM.
    /// POST /api/generate-queries — called by orch when creating a new batch
    async fn generate_queries(
        &self,
        region_name: String,
        count: u32,
    ) -> Result<GenerateQueriesResponse, Self::Error>;

    /// Resolve a chunk UUID to its full source details.
    /// GET /api/chunks/{chunk_id}/source
    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<Option<ChunkSource>, Self::Error>;
}
