use crate::InfraError;
use crate::env::BrainAtlasEnvInfra;
use crate::llm::OpenAiCompatibleClient;
use crate::llm_usage::BrainAtlasLlmUsage;
use crate::pg::BrainAtlasPostgresql;
use crate::s3::BrainAtlasS3;
use crate::vectordb::BrainAtlasVectorDB;
use domain::{
    ChunkSource, ExistingSummary, LlmCallOutcome, LlmPricing, LlmResponse, NewEmbedding,
    NewLlmCallUsage, NewRegionSummary, RetrievalScope, SimilarChunk, UsageAggregate,
    UsageAggregateFilter,
};
use services::infra::{
    EmbeddingGenerator, LlmClient, LlmPricingRepo, LlmUsageRepo, S3Storage, VectorDatabase,
};
use services::{EnvInfra, Postgres, Query, QueryResult};

pub struct BrainAtlasInfra {
    pg: BrainAtlasPostgresql,
    env: BrainAtlasEnvInfra,
    s3: BrainAtlasS3,
    llm: OpenAiCompatibleClient,
    vectordb: BrainAtlasVectorDB,
    llm_usage: BrainAtlasLlmUsage,
}

impl Default for BrainAtlasInfra {
    fn default() -> Self {
        Self::new()
    }
}

impl BrainAtlasInfra {
    pub fn new() -> Self {
        let env = BrainAtlasEnvInfra::new();

        // Read S3 bucket name from env. Empty-string env vars must not be
        // passed through unchanged: the SDK treats `Some("")` like a configured
        // value and produces dispatch failures instead of a clear error.
        let s3_bucket = env
            .get("S3_BUCKET")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .expect("S3_BUCKET env var is required and must be non-empty");

        // Optional S3 endpoint/credentials. When set (e.g. via docker-compose
        // for MinIO / NAS), the S3 client uses static credentials + path-style
        // addressing against the custom endpoint. When unset, it falls back to
        // the AWS default credential chain (EC2 instance profile) for native
        // AWS S3. `BrainAtlasS3::new` filters empty strings, so passing the raw
        // env values (including `${VAR:-}` empties from compose) is safe.
        let s3_endpoint = env.get("S3_ENDPOINT").ok();
        let s3_access_key = env.get("S3_ACCESS_KEY").ok();
        let s3_secret_key = env.get("S3_SECRET_KEY").ok();

        let pg = BrainAtlasPostgresql::new();
        let s3 = BrainAtlasS3::new(
            s3_endpoint.as_deref(),
            s3_access_key.as_deref(),
            s3_secret_key.as_deref(),
            s3_bucket,
        );
        let llm = OpenAiCompatibleClient::new();
        let vectordb = BrainAtlasVectorDB::new();
        let llm_usage = BrainAtlasLlmUsage::new();

        Self {
            pg,
            env,
            s3,
            llm,
            vectordb,
            llm_usage,
        }
    }
}
impl EnvInfra for BrainAtlasInfra {
    type Error = InfraError;
    fn get(&self, key: &str) -> Result<String, Self::Error> {
        self.env.get(key)
    }
}

#[async_trait::async_trait]
impl Postgres for BrainAtlasInfra {
    type Error = InfraError;

    async fn execute_query(
        &self,
        database_uri: &str,
        query: Query,
    ) -> Result<QueryResult, Self::Error> {
        self.pg.execute_query(database_uri, query).await
    }
}

#[async_trait::async_trait]
impl S3Storage for BrainAtlasInfra {
    type Error = InfraError;

    async fn download(&self, key: &str) -> Result<Option<String>, Self::Error> {
        self.s3.download(key).await
    }
}

#[async_trait::async_trait]
impl EmbeddingGenerator for BrainAtlasInfra {
    type Error = InfraError;

    async fn generate_embedding(
        &self,
        base_url: &str,
        api_key: &str,
        embedding_model: &str,
        text: &str,
    ) -> Result<LlmCallOutcome<Vec<f32>>, Self::Error> {
        self.llm
            .generate_embedding(base_url, api_key, embedding_model, text)
            .await
    }
}

#[async_trait::async_trait]
impl LlmClient for BrainAtlasInfra {
    type Error = InfraError;

    async fn summarize_with_tools(
        &self,
        base_url: &str,
        api_key: &str,
        chat_model: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<LlmCallOutcome<LlmResponse>, Self::Error> {
        self.llm
            .summarize_with_tools(base_url, api_key, chat_model, messages, tools)
            .await
    }

    async fn generate_queries(
        &self,
        base_url: &str,
        api_key: &str,
        chat_model: &str,
        region_name: &str,
        count: u32,
        acronym: Option<&str>,
        parent_name: Option<&str>,
        parent_acronym: Option<&str>,
    ) -> Result<LlmCallOutcome<Vec<String>>, Self::Error> {
        self.llm
            .generate_queries(
                base_url,
                api_key,
                chat_model,
                region_name,
                count,
                acronym,
                parent_name,
                parent_acronym,
            )
            .await
    }
}

#[async_trait::async_trait]
impl VectorDatabase for BrainAtlasInfra {
    type Error = InfraError;

    async fn insert_embeddings(
        &self,
        database_url: &str,
        embeddings: Vec<NewEmbedding>,
    ) -> Result<(), Self::Error> {
        self.vectordb
            .insert_embeddings(database_url, embeddings)
            .await
    }

    async fn insert_summary(
        &self,
        database_url: &str,
        summary: NewRegionSummary,
    ) -> Result<uuid::Uuid, Self::Error> {
        self.vectordb.insert_summary(database_url, summary).await
    }

    async fn check_content_hash(
        &self,
        database_url: &str,
        region_id: i32,
        hash: &str,
    ) -> Result<Option<ExistingSummary>, Self::Error> {
        self.vectordb
            .check_content_hash(database_url, region_id, hash)
            .await
    }

    async fn search_similar(
        &self,
        database_url: &str,
        query_embedding: Vec<f32>,
        retrieval_scope: RetrievalScope,
        top_k: usize,
    ) -> Result<Vec<SimilarChunk>, Self::Error> {
        self.vectordb
            .search_similar(database_url, query_embedding, retrieval_scope, top_k)
            .await
    }

    async fn update_summary_text(
        &self,
        database_url: &str,
        summary_id: uuid::Uuid,
        summary_text: &str,
    ) -> Result<(), Self::Error> {
        self.vectordb
            .update_summary_text(database_url, summary_id, summary_text)
            .await
    }

    async fn get_chunk_source(
        &self,
        database_url: &str,
        chunk_id: uuid::Uuid,
    ) -> Result<Option<ChunkSource>, Self::Error> {
        self.vectordb.get_chunk_source(database_url, chunk_id).await
    }
}

#[async_trait::async_trait]
impl LlmPricingRepo for BrainAtlasInfra {
    type Error = InfraError;

    async fn latest_for_model(
        &self,
        database_url: &str,
        model: &str,
    ) -> Result<Option<LlmPricing>, Self::Error> {
        self.llm_usage.latest_for_model(database_url, model).await
    }
}

#[async_trait::async_trait]
impl LlmUsageRepo for BrainAtlasInfra {
    type Error = InfraError;

    async fn record(&self, database_url: &str, row: NewLlmCallUsage) -> Result<(), Self::Error> {
        self.llm_usage.record(database_url, row).await
    }

    async fn aggregate(
        &self,
        database_url: &str,
        filter: UsageAggregateFilter,
    ) -> Result<UsageAggregate, Self::Error> {
        self.llm_usage.aggregate(database_url, filter).await
    }
}
