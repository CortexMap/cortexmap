use crate::InfraError;
use brainatlas_rpc_types::evals as brpc;
use domain::{ClaimsResponse, GroundednessVerdict, RubricScores};
use serde::Serialize;
use serde::de::DeserializeOwned;
use services::BrainatlasClient;

pub struct EvalsHttpClient {
    client: reqwest::Client,
}

impl EvalsHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client build"),
        }
    }

    async fn post_json<Req: Serialize, Res: DeserializeOwned>(
        &self,
        url: &str,
        body: &Req,
    ) -> Result<Res, InfraError> {
        tracing::debug!(url = url, "HTTP POST");
        let response = self.client.post(url).json(body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "(could not read response)".to_string());
            tracing::error!(url = url, status = %status, body = %body, "HTTP POST failed");
            return Err(InfraError::HttpStatus {
                status: status.as_u16(),
                body,
            });
        }
        Ok(response.json().await?)
    }
}

impl Default for EvalsHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl BrainatlasClient for EvalsHttpClient {
    type Error = InfraError;

    async fn extract_claims(
        &self,
        base_url: &str,
        req: brpc::ExtractClaimsRequest,
    ) -> Result<ClaimsResponse, Self::Error> {
        let url = format!("{}/brainatlas-be/api/llm/extract-claims", base_url.trim_end_matches('/'));
        self.post_json(&url, &req).await
    }

    async fn embed(
        &self,
        base_url: &str,
        req: brpc::EmbedRequest,
    ) -> Result<brpc::EmbedResponse, Self::Error> {
        let url = format!("{}/brainatlas-be/api/llm/embed", base_url.trim_end_matches('/'));
        self.post_json(&url, &req).await
    }

    async fn judge_groundedness(
        &self,
        base_url: &str,
        req: brpc::JudgeGroundednessRequest,
    ) -> Result<GroundednessVerdict, Self::Error> {
        let url = format!("{}/brainatlas-be/api/llm/judge-groundedness", base_url.trim_end_matches('/'));
        self.post_json(&url, &req).await
    }

    async fn judge_rubric(
        &self,
        base_url: &str,
        req: brpc::JudgeRubricRequest,
    ) -> Result<RubricScores, Self::Error> {
        let url = format!("{}/brainatlas-be/api/llm/judge-rubric", base_url.trim_end_matches('/'));
        self.post_json(&url, &req).await
    }

    async fn check_health(&self, base_url: &str) -> Result<(), Self::Error> {
        let url = format!("{}/brainatlas-be/health", base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(InfraError::NotFound);
        }
        Ok(())
    }
}
