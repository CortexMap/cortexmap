use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum ApiError<E: Error + Send + Sync + 'static> {
    #[error("app error: {0}")]
    AppError(#[source] E),

    #[error("missing or invalid id")]
    MissingOrInvalidId,
}
