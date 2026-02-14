use crate::{EnvInfra, Postgres, Query, QueryResult, ServiceError};
use app::ListBrainRegions;
use domain::RegionMapping;
use std::sync::Arc;

pub struct BrainAtlasListBrainRegions<I> {
    infra: Arc<I>,
}

impl<I> BrainAtlasListBrainRegions<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> ListBrainRegions for BrainAtlasListBrainRegions<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Postgres<Error = E> + EnvInfra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn list(&self) -> Result<Vec<RegionMapping>, Self::Error> {
        let db_uri = self
            .infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let result = self
            .infra
            .execute_query(&db_uri, Query::ListRegions)
            .await
            .map_err(ServiceError::InfraError)?;

        let QueryResult::Regions(regions) = result else {
            return Err(ServiceError::InvalidResult);
        };

        Ok(regions)
    }
}
