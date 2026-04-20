use domain::{
    BrainRegionEntry, ChunkSource, ClaimsResponse, ExistingSummary, GroundednessVerdict,
    LlmResponse, NewEmbedding, NewRegionSummary, RegionMapping, RubricScores, SimilarChunk,
};
use std::error::Error;
use uuid::Uuid;

/// List all brain regions
#[async_trait::async_trait]
pub trait ListBrainRegions: Send + Sync {
    type Error: Error + Send + Sync;

    async fn list(&self) -> Result<Vec<RegionMapping>, Self::Error>;
}

/// Get brain region information
#[async_trait::async_trait]
pub trait BrainRegionInfo: Send + Sync {
    type Error: Error + Send + Sync;
    async fn search(&self, id: Uuid) -> Result<Vec<BrainRegionEntry>, Self::Error>;
}

/// Text chunking (infallible)
pub trait Chunker: Send + Sync {
    fn chunk(&self, text: &str, chunk_size: usize, overlap: usize) -> Vec<String>;
}

/// LLM service for summarization and query generation
#[async_trait::async_trait]
pub trait LlmService: Send + Sync {
    type Error: Error + Send + Sync;

    /// Send a multi-turn chat with tool definitions, returning tool calls or final text.
    /// `chat_model_override` if Some, overrides the default/env model.
    async fn summarize_with_tools(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        chat_model_override: Option<&str>,
    ) -> Result<LlmResponse, Self::Error>;

    async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, Self::Error>;

    /// Split a summary into atomic factual claims tagged with their section heading.
    /// Used by the evals pipeline.
    async fn extract_claims(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model_override: Option<&str>,
    ) -> Result<ClaimsResponse, Self::Error>;

    /// Judge whether a single claim is supported by a list of candidate evidence chunks.
    /// `evidence_chunks` is a list of plain-text chunks; the judge returns 1-based indices
    /// into this list for the supporting subset.
    async fn judge_groundedness(
        &self,
        claim_text: &str,
        evidence_chunks: &[String],
        chat_model_override: Option<&str>,
    ) -> Result<GroundednessVerdict, Self::Error>;

    /// Score a full summary against the fixed five-criterion rubric.
    async fn judge_rubric(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model_override: Option<&str>,
    ) -> Result<RubricScores, Self::Error>;
}

/// Embedding generation service
#[async_trait::async_trait]
pub trait EmbeddingService: Send + Sync {
    type Error: Error + Send + Sync;

    /// Generate embedding for text.
    /// `model_override` if Some, overrides the default/env embedding model.
    async fn generate_embedding(
        &self,
        text: &str,
        model_override: Option<&str>,
    ) -> Result<Vec<f32>, Self::Error>;
}

/// S3 storage service
#[async_trait::async_trait]
pub trait S3Storage: Send + Sync {
    type Error: Error + Send + Sync;

    async fn download(&self, key: &str) -> Result<String, Self::Error>;
}

/// Vector database service
#[async_trait::async_trait]
pub trait VectorDatabase: Send + Sync {
    type Error: Error + Send + Sync;

    async fn check_content_hash(
        &self,
        region_id: i32,
        content_hash: &str,
    ) -> Result<Option<ExistingSummary>, Self::Error>;
    async fn insert_summary_with_embeddings(
        &self,
        summary: NewRegionSummary,
        embeddings: Vec<NewEmbedding>,
    ) -> Result<Uuid, Self::Error>;
    async fn search_similar(
        &self,
        query_embedding: Vec<f32>,
        region_id: i32,
        top_k: usize,
    ) -> Result<Vec<SimilarChunk>, Self::Error>;
    async fn update_summary_text(
        &self,
        summary_id: Uuid,
        summary_text: &str,
    ) -> Result<(), Self::Error>;
    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<Option<ChunkSource>, Self::Error>;
}

/// Combined services trait
pub trait Services:
    ListBrainRegions<Error = <Self as Services>::Error>
    + BrainRegionInfo<Error = <Self as Services>::Error>
    + Chunker
    + LlmService<Error = <Self as Services>::Error>
    + EmbeddingService<Error = <Self as Services>::Error>
    + S3Storage<Error = <Self as Services>::Error>
    + VectorDatabase<Error = <Self as Services>::Error>
{
    type Error: Error + Send + Sync;
}

impl<E, T> Services for T
where
    T: ListBrainRegions<Error = E>
        + BrainRegionInfo<Error = E>
        + Chunker
        + LlmService<Error = E>
        + EmbeddingService<Error = E>
        + S3Storage<Error = E>
        + VectorDatabase<Error = E>,
    E: Error + Send + Sync,
{
    type Error = E;
}
