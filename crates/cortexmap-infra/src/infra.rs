use crate::error::InfraError;
use bytes::Bytes;
use reqwest::Response;
use crate::{NewPaper, Paper};

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

