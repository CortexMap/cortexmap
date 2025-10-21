mod database;
mod http;
mod infra;

pub use database::*;

use crate::infra::StdInfra;
use cortexmap_infra::{InfraContext, InfraError};
use std::sync::Arc;

#[derive(derive_builder::Builder)]
pub struct StdInfraContext {
    pub database_url: String,
}

impl StdInfraContext {
    // maybe consume self?
    pub fn get(&self) -> Result<InfraContext<StdInfra>, InfraError> {
        // TODO: ideally this function should only be called ones
        // but it's easy to make mistakes here,
        // so maybe we could initiate this statically
        // and always return the same instance.
        Ok(InfraContext {
            infra: Arc::new(StdInfra::new(&self.database_url)?),
        })
    }
}
