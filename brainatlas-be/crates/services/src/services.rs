use crate::list_brain_regions::BrainAtlasListBrainRegions;
use crate::region_info::BrainAtlasRegionInfo;
use crate::chunker::TextChunker;
use crate::llm_service::BrainAtlasLlmService;
use crate::embedding_service::BrainAtlasEmbeddingService;
use crate::{Infra, ServiceError};
use app::{BrainRegionInfo, ListBrainRegions, Chunker, LlmService, EmbeddingService, S3Storage, VectorDatabase};
use domain::{BrainRegionEntry, RegionMapping, NewEmbedding, NewRegionSummary, ExistingSummary, SimilarChunk, ChunkSource, LlmResponse};
use std::sync::Arc;
use uuid::Uuid;

pub struct BrainAtlasServices<I> {
    infra: Arc<I>,
    brain_atlas_list_brain_regions: BrainAtlasListBrainRegions<I>,
    brain_atlas_region_info: BrainAtlasRegionInfo<I>,
    chunker: TextChunker,
    llm_service: BrainAtlasLlmService<I>,
    embedding_service: BrainAtlasEmbeddingService<I>,
}

impl<I: Infra> BrainAtlasServices<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let brain_atlas_list_brain_regions = BrainAtlasListBrainRegions::new(infra.clone());
        let brain_atlas_region_info = BrainAtlasRegionInfo::new(infra.clone());
        let llm_service = BrainAtlasLlmService::new(infra.clone());
        let embedding_service = BrainAtlasEmbeddingService::new(infra.clone());
        Self {
            infra,
            brain_atlas_list_brain_regions,
            brain_atlas_region_info,
            chunker: TextChunker::new(),
            llm_service,
            embedding_service,
        }
    }
}

// Implement Chunker trait
impl<I: Send + Sync> Chunker for BrainAtlasServices<I> {
    fn chunk(&self, text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
        self.chunker.chunk(text, chunk_size, overlap)
    }
}

// Implement LlmService trait
#[async_trait::async_trait]
impl<E, I> LlmService for BrainAtlasServices<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn summarize_with_tools(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        chat_model_override: Option<&str>,
    ) -> Result<LlmResponse, Self::Error> {
        self.llm_service.summarize_with_tools(messages, tools, chat_model_override).await
    }

    async fn generate_queries(&self, region_name: &str, count: u32) -> Result<Vec<String>, Self::Error> {
        self.llm_service.generate_queries(region_name, count).await
    }
}

// Implement EmbeddingService trait
#[async_trait::async_trait]
impl<E, I> EmbeddingService for BrainAtlasServices<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn generate_embedding(&self, text: &str, model_override: Option<&str>) -> Result<Vec<f32>, Self::Error> {
        self.embedding_service.generate_embedding(text, model_override).await
    }
}

#[async_trait::async_trait]
impl<E, I> ListBrainRegions for BrainAtlasServices<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn list(&self) -> Result<Vec<RegionMapping>, Self::Error> {
        self.brain_atlas_list_brain_regions.list().await
    }
}

#[async_trait::async_trait]
impl<E, I> BrainRegionInfo for BrainAtlasServices<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn search(&self, id: Uuid) -> Result<Vec<BrainRegionEntry>, Self::Error> {
        self.brain_atlas_region_info.search(id).await
    }
}

// Implement S3Storage trait
#[async_trait::async_trait]
impl<E, I> S3Storage for BrainAtlasServices<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn download(&self, key: &str) -> Result<String, Self::Error> {
        self.infra.download(key).await.map_err(ServiceError::InfraError)
    }
}

// Implement VectorDatabase trait
#[async_trait::async_trait]
impl<E, I> VectorDatabase for BrainAtlasServices<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn check_content_hash(&self, region_id: i32, content_hash: &str) -> Result<Option<ExistingSummary>, Self::Error> {
        let database_url = self.infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;
        
        self.infra
            .check_content_hash(&database_url, region_id, content_hash)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn insert_summary_with_embeddings(
        &self,
        summary: NewRegionSummary,
        mut embeddings: Vec<NewEmbedding>,
    ) -> Result<Uuid, Self::Error> {
        let database_url = self.infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;
        
        // 1. Insert the summary first
        let summary_id = self.infra
            .insert_summary(&database_url, summary)
            .await
            .map_err(ServiceError::InfraError)?;
        
        // 2. Update all embeddings with the summary_id
        for embedding in &mut embeddings {
            embedding.summary_id = summary_id;
        }
        
        // 3. Insert all embeddings
        self.infra
            .insert_embeddings(&database_url, embeddings)
            .await
            .map_err(ServiceError::InfraError)?;
        
        Ok(summary_id)
    }

    async fn search_similar(
        &self,
        query_embedding: Vec<f32>,
        region_id: i32,
        top_k: usize,
    ) -> Result<Vec<SimilarChunk>, Self::Error> {
        let database_url = self
            .infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .search_similar(&database_url, query_embedding, region_id, top_k)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn update_summary_text(
        &self,
        summary_id: Uuid,
        summary_text: &str,
    ) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .update_summary_text(&database_url, summary_id, summary_text)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_chunk_source(
        &self,
        chunk_id: Uuid,
    ) -> Result<Option<ChunkSource>, Self::Error> {
        let database_url = self
            .infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_chunk_source(&database_url, chunk_id)
            .await
            .map_err(ServiceError::InfraError)
    }
}
