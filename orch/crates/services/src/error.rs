#[derive(Debug, thiserror::Error)]
pub enum ServiceError<E: std::error::Error + Send + Sync + 'static> {
    #[error("infra error: {0}")]
    InfraError(#[source] E),

    #[error("invalid query result")]
    InvalidResult,

    #[error("region not found")]
    NotFound,

    #[error("configuration key not found: {key}")]
    ConfigNotFound { key: String },

    #[error("no S3 keys found for task")]
    NoS3Keys,

    #[error("task already processed")]
    AlreadyProcessed,

    #[error("failed to parse configuration value")]
    ConfigParseFailed,
    
    #[error("invalid configuration: {reason}")]
    InvalidConfig { reason: String },
}
