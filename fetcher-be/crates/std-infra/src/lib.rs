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
    ///
    /// Empty env-var values are treated as absent: passing `Some("")` to the
    /// AWS SDK causes `endpoint_url`/`credentials_provider` to be called with
    /// an empty string, which breaks request dispatch. Filter them out here.
    #[allow(clippy::result_large_err)]
    pub fn from_env() -> Result<Self, InfraError> {
        let env = FetcherEnvInfra::new();
        let non_empty = |key: &str| -> Option<String> {
            env.get_env_var(key)
                .ok()
                .filter(|s| !s.trim().is_empty())
        };
        Ok(Self {
            database_url: env.get_env_var("DATABASE_URL")?,
            endpoint: non_empty("S3_ENDPOINT"),
            access_key: non_empty("S3_ACCESS_KEY"),
            secret_key: non_empty("S3_SECRET_KEY"),
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
