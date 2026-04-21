use api::{Evals, build_router};
use axum::Router;
use infra::{EvalsInfra, InfraError};
use std::sync::Arc;

pub struct EvalsServer {
    api: Arc<Evals<EvalsInfra, EvalsInfra, InfraError>>,
}

impl Clone for EvalsServer {
    fn clone(&self) -> Self {
        Self {
            api: self.api.clone(),
        }
    }
}

impl EvalsServer {
    pub fn new(api: Arc<Evals<EvalsInfra, EvalsInfra, InfraError>>) -> Self {
        Self { api }
    }

    pub fn into_router(self) -> Router {
        build_router(self.api)
    }
}
