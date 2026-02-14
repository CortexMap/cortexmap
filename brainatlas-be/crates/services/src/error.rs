#[derive(Debug, thiserror::Error)]
pub enum ServiceError<E: std::error::Error + Send + Sync + 'static> {
    #[error("infra error: {0}")]
    InfraError(#[source] E),

    #[error("Invalid query result")]
    InvalidResult,
}
