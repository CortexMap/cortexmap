mod database;
mod http;
mod infra;
mod s3;
mod task_queue;

pub use database::*;
pub use infra::StdInfra; // Make StdInfra public for testing

use cortexmap_infra::{InfraContext, InfraError};
use std::sync::Arc;

#[derive(derive_builder::Builder)]
pub struct StdInfraContext {
    pub database_url: String,
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
}

impl StdInfraContext {
    // Note: Creates a new InfraContext on each call.
    // Currently simple but could be optimized to use a singleton pattern
    // to ensure only one instance is created across the application.
    pub fn get(&self) -> Result<InfraContext<StdInfra>, InfraError> {
        Ok(InfraContext {
            infra: Arc::new(StdInfra::new(
                &self.database_url,
                &self.endpoint,
                &self.access_key,
                &self.secret_key,
                &self.bucket,
            )?),
        })
    }
}
