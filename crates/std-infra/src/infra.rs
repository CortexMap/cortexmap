use crate::http::StdHttpInfra;
use bytes::Bytes;
use cortexmap_infra::{HttpInfra, InfraError};
use reqwest::Response;

pub struct StdInfra {
    http_infra: StdHttpInfra,
}

impl StdInfra {
    pub fn new() -> Self {
        let http_infra = StdHttpInfra::new();
        Self { http_infra }
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
