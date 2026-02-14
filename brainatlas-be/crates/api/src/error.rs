#[derive(Debug, thiserror::Error)]
pub enum ApiError<E> {
    #[error("app error: {0}")]
    AppError(#[source] E),

    #[error("not implemented")]
    NotImplemented,

    #[error("missing or invalid id")]
    MissingOrInvalidId,
}
