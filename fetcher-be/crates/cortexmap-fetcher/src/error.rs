use cortexmap_infra::InfraError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Infra Error: {0}")]
    InfraError(#[from] Box<InfraError>),

    #[error("Reqwest Error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("Serde Error: {0}")]
    SerdeError(#[from] serde_json::Error),

    #[error("Join Error: {0}")]
    JoinError(tokio::task::JoinError),

    #[error("Invalid PDF Source: {0}")]
    InvalidPdfSource(String),

    #[error("Not Found: {0}")]
    NotFound(String),
}

impl From<InfraError> for FetchError {
    fn from(error: InfraError) -> Self {
        FetchError::InfraError(Box::new(error))
    }
}
