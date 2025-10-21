use crate::error::InfraError;
use crate::{NewPaper, Paper};
use bytes::Bytes;
use futures::Stream;
use reqwest::Response;
use std::path::PathBuf;
use std::pin::Pin;

#[async_trait::async_trait]
pub trait HttpInfra {
    // TODO: add some wrapper instead of using reqwest::Response
    async fn get(&self, url: &str) -> Result<Response, InfraError>;
    async fn post(&self, url: &str, body: Option<Bytes>) -> Result<Response, InfraError>;
}

#[async_trait::async_trait]
pub trait DatabaseInfra {
    /// Insert a new paper into the database
    async fn insert_paper(&self, new_paper: NewPaper) -> Result<Paper, InfraError>;
}

#[async_trait::async_trait]
pub trait S3Infra {
    async fn put(
        &self,
        key: &str,
        content_type: &str,
        content: Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>,
    ) -> Result<(), InfraError>;
}
