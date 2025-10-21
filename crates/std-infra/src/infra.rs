use crate::StdDatabaseInfra;
use crate::http::StdHttpInfra;
use bytes::Bytes;
use cortexmap_infra::{DatabaseInfra, HttpInfra, InfraError, NewPaper, Paper};
use reqwest::Response;

pub struct StdInfra {
    http_infra: StdHttpInfra,
    db_infra: StdDatabaseInfra,
}

impl StdInfra {
    pub fn new(database_url: &str) -> Result<Self, InfraError> {
        let http_infra = StdHttpInfra::new();
        let db_infra = StdDatabaseInfra::new(database_url)?;
        Ok(Self {
            http_infra,
            db_infra,
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
