use domain::{BrainRegionEntry, RegionMapping, NewEmbedding, NewRegionSummary, ExistingSummary};
use uuid::Uuid;
use std::error::Error;

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
    
    async fn summarize(&self, chunks: Vec<&str>) -> Result<String, Self::Error>;
    async fn generate_queries(&self, region_name: &str, count: u32) -> Result<Vec<String>, Self::Error>;
}

/// Embedding generation service
#[async_trait::async_trait]
pub trait EmbeddingService: Send + Sync {
    type Error: Error + Send + Sync;
    
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Self::Error>;
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
    
    async fn check_content_hash(&self, region_id: i32, content_hash: &str) -> Result<Option<ExistingSummary>, Self::Error>;
    async fn insert_summary_with_embeddings(
        &self,
        summary: NewRegionSummary,
        embeddings: Vec<NewEmbedding>,
    ) -> Result<Uuid, Self::Error>;
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
