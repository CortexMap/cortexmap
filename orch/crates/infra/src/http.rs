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
        
        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_else(|_| "(could not read response)".to_string());
            tracing::error!(
                url = url,
                status = %status,
                response = %error_body,
                "HTTP GET failed"
            );
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
        
        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_else(|_| "(could not read response)".to_string());
            tracing::error!(
                url = url,
                status = %status,
                response = %error_body,
                "HTTP POST failed"
            );
            return Err(InfraError::NotFound);
        }
        
        Ok(response.json().await?)
    }
    
    async fn check_health(&self, base_url: &str, service_name: &str) -> Result<(), Self::Error> {
        let url = format!("{}/health", base_url);
        
        let response = self.client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| {
                tracing::error!(
                    service = service_name,
                    base_url = base_url,
                    error = %e,
                    "Health check failed - could not connect to service"
                );
                InfraError::from(e)
            })?;
        
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            tracing::error!(
                service = service_name,
                base_url = base_url,
                status = %status,
                body = %body,
                "Health check failed - service returned non-success status"
            );
            return Err(InfraError::NotFound);
        }
        
        tracing::info!(
            service = service_name,
            base_url = base_url,
            "Health check passed"
        );
        
        Ok(())
    }
}
