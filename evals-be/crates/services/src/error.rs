use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError<E: Error + Send + Sync + 'static> {
    #[error("infra error: {0}")]
    InfraError(#[source] E),

    #[error("not found")]
    NotFound,

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("brainatlas call failed: {0}")]
    Brainatlas(String),

    #[error("{0}")]
    Other(String),
}
