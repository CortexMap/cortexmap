use thiserror::Error;

#[derive(Error, Debug)]
pub enum InfraError {
    #[error("Http error: {0}")]
    HttpError(#[from] reqwest::Error),
}
