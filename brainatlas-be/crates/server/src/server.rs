use api::BrainRegionApi;
use rpc_types::{
    BrainRegionListResponse, BrainRegionService, SearchBrainRegionRequest,
    SearchBrainRegionResponse, StatusRequest, StatusResponse,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub struct BrainAtlasServer<A> {
    api: Arc<A>,
}

impl<A> BrainAtlasServer<A> {
    pub fn new(api: Arc<A>) -> Self {
        Self { api }
    }
}

#[tonic::async_trait]
impl<A> BrainRegionService for BrainAtlasServer<A>
where
    A: BrainRegionApi + 'static,
{
    async fn search_brain_region(
        &self,
        request: Request<SearchBrainRegionRequest>,
    ) -> Result<Response<SearchBrainRegionResponse>, Status> {
        let id_str = request
            .into_inner()
            .id
            .ok_or_else(|| Status::invalid_argument("missing id"))?
            .value;

        let id = uuid::Uuid::parse_str(&id_str)
            .map_err(|e| Status::invalid_argument(format!("invalid uuid: {e}")))?;

        let entry = self
            .api
            .search_brain_region(id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(SearchBrainRegionResponse {
            entry: Some(entry.into()),
        }))
    }

    async fn list_brain_regions(
        &self,
        _request: Request<()>,
    ) -> Result<Response<BrainRegionListResponse>, Status> {
        let regions = self
            .api
            .list_brain_regions()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(BrainRegionListResponse {
            regions: regions.into_iter().map(Into::into).collect(),
        }))
    }

    async fn status(
        &self,
        request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let id_str = request
            .into_inner()
            .id
            .ok_or_else(|| Status::invalid_argument("missing id"))?
            .value;

        let id = uuid::Uuid::parse_str(&id_str)
            .map_err(|e| Status::invalid_argument(format!("invalid uuid: {e}")))?;

        let status = self
            .api
            .status(id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let proto_status: rpc_types::Status = status.into();
        Ok(Response::new(StatusResponse {
            status: proto_status as i32,
        }))
    }
}
