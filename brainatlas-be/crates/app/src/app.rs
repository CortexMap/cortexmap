use crate::{AppError, Services};
use domain::RegionMapping;
use std::sync::Arc;

pub struct BrainAtlasApp<S> {
    services: Arc<S>,
}

impl<E, S> BrainAtlasApp<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E>,
{
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }

    pub async fn list(&self) -> Result<Vec<RegionMapping>, AppError<E>> {
        self.services.list().await.map_err(AppError::ServiceError)
    }
}
