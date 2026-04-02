mod database;
mod env;
mod http;
mod infra;
pub mod redis_infra;
mod s3;
mod task_queue;

pub use database::*;
pub use env::*;
pub use infra::StdInfra; // Make StdInfra public for testing
pub use redis_infra::StdRedisInfra;

use cortexmap_infra::{EnvInfra, InfraContext, InfraError};
use std::sync::Arc;

#[derive(derive_builder::Builder)]
pub struct StdInfraContext {
    pub database_url: String,
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    pub redis_url: String,
}

impl StdInfraContext {
    /// Create from runtime environment variables (collected once, reused).
    pub fn from_env() -> Result<Self, InfraError> {
        let env = FetcherEnvInfra::new();
        Ok(Self {
            database_url: env.get_env_var("DATABASE_URL")?,
            endpoint: env.get_env_var("S3_ENDPOINT")?,
            access_key: env.get_env_var("S3_ACCESS_KEY")?,
            secret_key: env.get_env_var("S3_SECRET_KEY")?,
            bucket: env.get_env_var("S3_BUCKET")?,
            redis_url: env.get_env_var("REDIS_URL")?,
        })
    }

    pub fn get(&self) -> Result<InfraContext<StdInfra>, InfraError> {
        Ok(InfraContext {
            infra: Arc::new(StdInfra::new(
                &self.database_url,
                &self.endpoint,
                &self.access_key,
                &self.secret_key,
                &self.bucket,
                &self.redis_url,
            )?),
        })
    }
}
