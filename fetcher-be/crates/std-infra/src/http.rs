use bytes::Bytes;
use cortexmap_infra::{HttpInfra, InfraError};
use reqwest::Response;

pub struct StdHttpInfra {
    client: reqwest::Client,
}

impl StdHttpInfra {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl HttpInfra for StdHttpInfra {
    async fn get(&self, url: &str) -> Result<Response, InfraError> {
        Ok(self.client.get(url).send().await?.error_for_status()?)
    }

    async fn post(&self, url: &str, body: Option<Bytes>) -> Result<Response, InfraError> {
        let mut req = self.client.post(url);
        if let Some(body) = body {
            req = req.body(body);
        }
        Ok(req.send().await?.error_for_status()?)
    }
}
