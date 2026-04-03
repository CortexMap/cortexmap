#[derive(Debug, thiserror::Error)]
pub enum AppError<E: std::error::Error + Send + Sync + 'static> {
    #[error("service error: {0}")]
    ServiceError(#[source] E),

    #[error("Invalid query result")]
    InvalidResult,

    #[error("not found")]
    NotFound,

    #[error("LLM exceeded maximum tool call iterations ({0})")]
    MaxToolCallsExceeded(usize),
}
