use crate::{ApiError, BrainRegionApi};
use app::{AppError, BrainAtlasApp, Services};
use domain::ChunkSource;
use domain::rpc_types;
use domain::rpc_types::{
    BrainRegionListResponse, GenerateQueriesResponse, PaperMetadata, ProcessRegionResponse,
    SearchBrainRegionResponse, StatusResponse,
};
use std::sync::Arc;
use uuid::Uuid;

pub struct BrainAtlasApi<S> {
    services: Arc<S>,
}

impl<S> BrainAtlasApi<S> {
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }
}

impl<S: Services + 'static> BrainAtlasApi<S> {
    fn app(&self) -> BrainAtlasApp<S> {
        BrainAtlasApp::new(self.services.clone())
    }
}

#[async_trait::async_trait]
impl<E, S> BrainRegionApi for BrainAtlasApi<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E> + 'static,
{
    type Error = ApiError<AppError<E>>;

    async fn search_brain_region(
        &self,
        id: Option<Uuid>,
    ) -> Result<SearchBrainRegionResponse, Self::Error> {
        let id = id.ok_or(ApiError::MissingOrInvalidId)?;
        let entries = self.app().search(id).await.map_err(ApiError::AppError)?;
        Ok(SearchBrainRegionResponse {
            entries: entries.into_iter().map(Into::into).collect(),
        })
    }

    async fn list_brain_regions(&self) -> Result<BrainRegionListResponse, Self::Error> {
        let regions = self.app().list().await.map_err(ApiError::AppError)?;
        Ok(BrainRegionListResponse {
            regions: regions.into_iter().map(Into::into).collect(),
        })
    }

    async fn status(&self, _id: Uuid) -> Result<StatusResponse, Self::Error> {
        Err(ApiError::NotImplemented)
    }

    async fn process_region(
        &self,
        region_id: Option<Uuid>,
        batch_id: Option<Uuid>,
        s3_keys: Vec<String>,
        paper_metadata: Vec<PaperMetadata>,
        chat_model: Option<String>,
        embedding_model: Option<String>,
        skip_summarization: bool,
    ) -> Result<ProcessRegionResponse, Self::Error> {
        // Validate region_id and batch_id are present
        let region_uuid = region_id.ok_or(ApiError::MissingOrInvalidId)?;
        let batch_uuid = batch_id.ok_or(ApiError::MissingOrInvalidId)?;

        // Call the processing pipeline
        let summary_id = self
            .app()
            .process_region(
                region_uuid,
                batch_uuid,
                s3_keys,
                paper_metadata,
                chat_model,
                embedding_model,
                skip_summarization,
            )
            .await
            .map_err(ApiError::AppError)?;

        let detail = if skip_summarization {
            format!(
                "Successfully chunked and embedded for summary {}",
                summary_id
            )
        } else {
            format!("Successfully created summary {}", summary_id)
        };

        Ok(ProcessRegionResponse {
            region_id: Some(rpc_types::Uuid {
                value: region_uuid.to_string(),
            }),
            detail,
        })
    }

    async fn generate_queries(
        &self,
        region_name: String,
        count: u32,
    ) -> Result<GenerateQueriesResponse, Self::Error> {
        // Call the LLM to generate queries
        let queries = self
            .app()
            .generate_queries(&region_name, count)
            .await
            .map_err(ApiError::AppError)?;

        Ok(GenerateQueriesResponse { queries })
    }

    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<Option<ChunkSource>, Self::Error> {
        self.app()
            .get_chunk_source(chunk_id)
            .await
            .map_err(ApiError::AppError)
    }
}
