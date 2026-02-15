use crate::InfraError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use services::HttpClient as HttpClientTrait;

pub struct OrchHttpClient {
    client: reqwest::Client,
}

impl OrchHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl HttpClientTrait for OrchHttpClient {
    type Error = InfraError;
    
    async fn get<T: DeserializeOwned + Send>(&self, url: &str) -> Result<T, Self::Error> {
        let response = self.client.get(url).send().await?;
        
        if !response.status().is_success() {
            // Consume response body to avoid connection issues
            let _ = response.text().await;
            return Err(InfraError::NotFound);
        }
        
        Ok(response.json().await?)
    }
    
    async fn post<Req: Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
        &self,
        url: &str,
        body: &Req,
    ) -> Result<Res, Self::Error> {
        let response = self.client.post(url).json(body).send().await?;
        
        if !response.status().is_success() {
            // Consume response body to avoid connection issues
            let _ = response.text().await;
            return Err(InfraError::NotFound);
        }
        
        Ok(response.json().await?)
    }
}
