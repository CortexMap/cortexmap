use crate::error::InfraError;
use bytes::Bytes;
use reqwest::Response;

#[async_trait::async_trait]
pub trait HttpInfra {
    // TODO: add some wrapper instead of using reqwest::Response
    async fn get(&self, url: &str) -> Result<Response, InfraError>;
    async fn post(&self, url: &str, body: Option<Bytes>) -> Result<Response, InfraError>;
}
