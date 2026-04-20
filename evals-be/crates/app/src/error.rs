use services::ServiceError;
use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum AppError<E: Error + Send + Sync + 'static> {
    #[error("service error: {0}")]
    Service(#[source] ServiceError<E>),

    #[error("summary not found")]
    SummaryNotFound,

    #[error("missing required env var: {0}")]
    MissingEnv(String),

    #[error("invalid config value for {key}: {value}")]
    InvalidConfig { key: String, value: String },

    #[error("invalid argument: {0}")]
    InvalidArg(String),
}

impl<E: Error + Send + Sync + 'static> From<ServiceError<E>> for AppError<E> {
    fn from(e: ServiceError<E>) -> Self {
        AppError::Service(e)
    }
}
