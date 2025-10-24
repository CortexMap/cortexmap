mod database;
mod http;
mod infra;
mod s3;

pub use database::*;

use crate::infra::StdInfra;
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
    // maybe consume self?
    pub fn get(&self) -> Result<InfraContext<StdInfra>, InfraError> {
        // TODO: ideally this function should only be called ones
        // but it's easy to make mistakes here,
        // so maybe we could initiate this statically
        // and always return the same instance.
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
