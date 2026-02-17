use crate::{EnvInfra, Postgres, Query, QueryResult, ServiceError};
use app::BrainRegionInfo;
use domain::BrainRegionEntry;
use std::sync::Arc;
use uuid::Uuid;

pub struct BrainAtlasRegionInfo<I> {
    infra: Arc<I>,
}

impl<I> BrainAtlasRegionInfo<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> BrainRegionInfo for BrainAtlasRegionInfo<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Postgres<Error = E> + EnvInfra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn search(&self, id: Uuid) -> Result<Vec<BrainRegionEntry>, Self::Error> {
        let db_uri = self
            .infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let QueryResult::Region(entries) = self
            .infra
            .execute_query(&db_uri, Query::GetRegionById(id))
            .await
            .map_err(ServiceError::InfraError)?
        else {
            return Err(ServiceError::InvalidResult);
        };

        Ok(entries)
    }
}
