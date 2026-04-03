use domain::{
    BrainRegionEntry, ChunkSource, ExistingSummary, LlmResponse, NewEmbedding, NewRegionSummary,
    RegionMapping, SimilarChunk,
};
use uuid::Uuid;

/// All queries the service layer can issue against Postgres.
pub enum Query {
    /// Fetch all rows from `region_mapping`, ordered by `structure_order`.
    ListRegions,
    /// Fetch a single region by UUID primary key.
    GetRegionById(Uuid),
    /// Check whether a row with the given UUID exists.
    RegionExists(Uuid),
}

/// Typed results returned by each query variant.
pub enum QueryResult {
    Regions(Vec<RegionMapping>),
    Region(Vec<BrainRegionEntry>),
    Exists(bool),
}

/// Postgres infra trait — accepts a typed query and executes it.
/// Connection management and DB row conversion are entirely internal to the implementation.
#[async_trait::async_trait]
pub trait Postgres: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn execute_query(
        &self,
        database_uri: &str,
        query: Query,
    ) -> Result<QueryResult, Self::Error>;
}

pub trait EnvInfra {
    type Error: std::error::Error + Send + Sync + 'static;
    fn get(&self, key: &str) -> Result<String, Self::Error>;
}

/// Blanket: any `T: Postgres` automatically satisfies `Infra`.
pub trait Infra:
    Postgres<Error = <Self as Infra>::Error>
    + EnvInfra<Error = <Self as Infra>::Error>
    + S3Storage<Error = <Self as Infra>::Error>
    + EmbeddingGenerator<Error = <Self as Infra>::Error>
    + LlmClient<Error = <Self as Infra>::Error>
    + VectorDatabase<Error = <Self as Infra>::Error>
{
    type Error: std::error::Error + Send + Sync + 'static;
}

impl<E, T> Infra for T
where
    T: Postgres<Error = E>
        + EnvInfra<Error = E>
        + S3Storage<Error = E>
        + EmbeddingGenerator<Error = E>
        + LlmClient<Error = E>
        + VectorDatabase<Error = E>,
    E: std::error::Error + Send + Sync + 'static,
{
    type Error = E;
}

/// S3 credentials for self-hosted S3-compatible storage
#[derive(Debug, Clone)]
pub struct S3Creds {
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
}

/// S3 storage access
#[async_trait::async_trait]
pub trait S3Storage: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Download file from S3 as UTF-8 string (reads credentials from env internally)
    async fn download(&self, key: &str) -> Result<String, Self::Error>;
}

/// Embedding generation
#[async_trait::async_trait]
pub trait EmbeddingGenerator: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Generate embedding for text chunk
    async fn generate_embedding(
        &self,
        api_key: &str,
        embedding_model: &str,
        text: &str,
    ) -> Result<Vec<f32>, Self::Error>;
}

/// LLM client for text generation
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Send a chat completion request with tool definitions, returning either
    /// tool calls the LLM wants to make or the final text response.
    async fn summarize_with_tools(
        &self,
        api_key: &str,
        chat_model: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<LlmResponse, Self::Error>;

    /// Generate search queries for a brain region
    async fn generate_queries(
        &self,
        api_key: &str,
        chat_model: &str,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, Self::Error>;
}

/// Vector database operations
#[async_trait::async_trait]
pub trait VectorDatabase: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Insert embeddings in bulk
    async fn insert_embeddings(
        &self,
        database_url: &str,
        embeddings: Vec<NewEmbedding>,
    ) -> Result<(), Self::Error>;

    /// Insert region summary and return ID
    async fn insert_summary(
        &self,
        database_url: &str,
        summary: NewRegionSummary,
    ) -> Result<Uuid, Self::Error>;

    /// Check if content hash already exists
    async fn check_content_hash(
        &self,
        database_url: &str,
        region_id: i32,
        content_hash: &str,
    ) -> Result<Option<ExistingSummary>, Self::Error>;

    /// Search for similar chunks by embedding vector, scoped to a region
    async fn search_similar(
        &self,
        database_url: &str,
        query_embedding: Vec<f32>,
        region_id: i32,
        top_k: usize,
    ) -> Result<Vec<SimilarChunk>, Self::Error>;

    /// Update the summary text for an existing summary record
    async fn update_summary_text(
        &self,
        database_url: &str,
        summary_id: Uuid,
        summary_text: &str,
    ) -> Result<(), Self::Error>;

    /// Get full source details for a chunk by its UUID
    async fn get_chunk_source(
        &self,
        database_url: &str,
        chunk_id: Uuid,
    ) -> Result<Option<ChunkSource>, Self::Error>;
}
