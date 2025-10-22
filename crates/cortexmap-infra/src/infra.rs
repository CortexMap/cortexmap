use std::fmt::{Display, Formatter};
use crate::error::InfraError;
use crate::{NewPaper, Paper};
use bytes::Bytes;
use futures::Stream;
use reqwest::Response;
use std::pin::Pin;

pub enum ContentType {
    Text,
    Pdf,
}

impl Display for ContentType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentType::Text => {
                write!(f, "text/plain")
            }
            ContentType::Pdf => {
                write!(f, "application/pdf")
            }
        }
    }
}

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
    async fn put_s3(
        &self,
        key: &str,
        content_type: ContentType,
        content: Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>,
    ) -> Result<(), InfraError>;
}
