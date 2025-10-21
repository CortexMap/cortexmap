use crate::StdDatabaseInfra;
use crate::http::StdHttpInfra;
use crate::s3::StdS3Infra;
use bytes::Bytes;
use cortexmap_infra::{DatabaseInfra, HttpInfra, InfraError, NewPaper, Paper, S3Infra};
use futures::Stream;
use reqwest::Response;
use std::pin::Pin;

pub struct StdInfra {
    http_infra: StdHttpInfra,
    db_infra: StdDatabaseInfra,
    s3_infra: StdS3Infra,
}

impl StdInfra {
    pub fn new(
        database_url: &str,
        endpoint: &str,
        access_key: &str,
        secret_key: &str,
        bucket: &str,
    ) -> Result<Self, InfraError> {
        let http_infra = StdHttpInfra::new();
        let db_infra = StdDatabaseInfra::new(database_url)?;
        let s3_infra = StdS3Infra::new(endpoint, access_key, secret_key, bucket);
        Ok(Self {
            http_infra,
            db_infra,
            s3_infra,
        })
    }
}

#[async_trait::async_trait]
impl HttpInfra for StdInfra {
    async fn get(&self, url: &str) -> Result<Response, InfraError> {
        self.http_infra.get(url).await
    }

    async fn post(&self, url: &str, body: Option<Bytes>) -> Result<Response, InfraError> {
        self.http_infra.post(url, body).await
    }
}

#[async_trait::async_trait]
impl DatabaseInfra for StdInfra {
    async fn insert_paper(&self, new_paper: NewPaper) -> Result<Paper, InfraError> {
        self.db_infra.insert_paper(new_paper).await
    }
}

#[async_trait::async_trait]
impl S3Infra for StdInfra {
    async fn put(
        &self,
        key: &str,
        content_type: &str,
        content: Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>,
    ) -> Result<(), InfraError> {
        self.s3_infra.put(key, content_type, content).await
    }
}
