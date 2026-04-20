#[derive(Debug, thiserror::Error)]
pub enum ServiceError<E: std::error::Error + Send + Sync + 'static> {
    #[error("infra error: {0}")]
    InfraError(#[source] E),

    #[error("Invalid query result")]
    InvalidResult,

    #[error("region not found")]
    NotFound,

    #[error("{0}")]
    Other(String),
}
