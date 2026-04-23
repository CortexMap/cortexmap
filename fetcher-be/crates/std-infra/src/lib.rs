mod database;
mod env;
mod http;
mod infra;
mod s3;
mod task_queue;

pub use database::*;
pub use env::*;
pub use infra::StdInfra; // Make StdInfra public for testing

use cortexmap_infra::{EnvInfra, InfraContext, InfraError};
use std::sync::Arc;

#[derive(derive_builder::Builder)]
pub struct StdInfraContext {
    pub database_url: String,
    #[builder(default)]
    pub endpoint: Option<String>,
    #[builder(default)]
    pub access_key: Option<String>,
    #[builder(default)]
    pub secret_key: Option<String>,
    pub bucket: String,
}

impl StdInfraContext {
    /// Create from runtime environment variables (collected once, reused).
    #[allow(clippy::result_large_err)]
    pub fn from_env() -> Result<Self, InfraError> {
        let env = FetcherEnvInfra::new();
        Ok(Self {
            database_url: env.get_env_var("DATABASE_URL")?,
            endpoint: env.get_env_var("S3_ENDPOINT").ok(),
            access_key: env.get_env_var("S3_ACCESS_KEY").ok(),
            secret_key: env.get_env_var("S3_SECRET_KEY").ok(),
            bucket: env.get_env_var("S3_BUCKET")?,
        })
    }

    #[allow(clippy::result_large_err)]
    pub fn get(&self) -> Result<InfraContext<StdInfra>, InfraError> {
        Ok(InfraContext {
            infra: Arc::new(StdInfra::new(
                &self.database_url,
                self.endpoint.as_deref(),
                self.access_key.as_deref(),
                self.secret_key.as_deref(),
                &self.bucket,
            )?),
        })
    }
}
