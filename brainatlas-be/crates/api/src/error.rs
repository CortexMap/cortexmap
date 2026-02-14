#[derive(Debug, thiserror::Error)]
pub enum ApiError<E> {
    #[error("app error: {0}")]
    AppError(#[source] E)
}
