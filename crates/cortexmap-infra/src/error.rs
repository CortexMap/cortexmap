use thiserror::Error;

#[derive(Error, Debug)]
pub enum InfraError {
    #[error("Http error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Database error: {0}")]
    Database(#[from] diesel::result::Error),

    #[error("Join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("Pool error: {0}")]
    R2D2PoolError(#[from] diesel::r2d2::PoolError),
}
