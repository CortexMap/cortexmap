use aws_sdk_s3::config::http::HttpResponse;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::put_object::PutObjectError;
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

    #[error("Put object error: {0}")]
    PutObjectError(#[from] SdkError<PutObjectError, HttpResponse>),
}
