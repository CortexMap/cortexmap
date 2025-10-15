use cortexmap_infra::InfraError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Infra Error: {0}")]
    InfraError(#[from] InfraError),

    #[error("Reqwest Error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("Serde Error: {0}")]
    SerdeError(#[from] serde_json::Error),

    #[error("Join Error: {0}")]
    JoinError(String),

    #[error("Invalid PDF Source: {0}")]
    InvalidPdfSource(String),
}
