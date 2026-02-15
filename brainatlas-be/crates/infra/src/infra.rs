use crate::InfraError;
use crate::env::BrainAtlasEnvInfra;
use crate::llm::OpenRouterClient;
use crate::pg::BrainAtlasPostgresql;
use crate::s3::BrainAtlasS3;
use crate::vectordb::BrainAtlasVectorDB;
use domain::{ExistingSummary, NewEmbedding, NewRegionSummary};
use services::infra::{EmbeddingGenerator, LlmClient, S3Storage, VectorDatabase};
use services::{EnvInfra, Postgres, Query, QueryResult};

pub struct BrainAtlasInfra {
    pg: BrainAtlasPostgresql,
    env: BrainAtlasEnvInfra,
    s3: BrainAtlasS3,
    llm: OpenRouterClient,
    vectordb: BrainAtlasVectorDB,
}

impl BrainAtlasInfra {
    pub fn new() -> Self {
        let pg = BrainAtlasPostgresql::new();
        let env = BrainAtlasEnvInfra::new();
        let s3 = BrainAtlasS3::new();

        let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
        let llm = OpenRouterClient::new(api_key);

        let vectordb = BrainAtlasVectorDB::new();

        Self {
            pg,
            env,
            s3,
            llm,
            vectordb,
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

    async fn download(&self, key: &str) -> Result<String, Self::Error> {
        self.s3.download(key).await
    }
}

#[async_trait::async_trait]
impl EmbeddingGenerator for BrainAtlasInfra {
    type Error = InfraError;

    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Self::Error> {
        self.llm.generate_embedding(text).await
    }
}

#[async_trait::async_trait]
impl LlmClient for BrainAtlasInfra {
    type Error = InfraError;

    async fn summarize(&self, chunks: Vec<&str>) -> Result<String, Self::Error> {
        self.llm.summarize(chunks).await
    }

    async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, Self::Error> {
        self.llm.generate_queries(region_name, count).await
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
}
