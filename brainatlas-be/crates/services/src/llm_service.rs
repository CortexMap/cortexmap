/// LLM service wrapper for text generation tasks
use crate::{EnvInfra, LlmClient, ServiceError};
use domain::{ClaimsResponse, GroundednessVerdict, LlmResponse, RubricScores};
use std::sync::Arc;
use tracing::warn;

pub struct BrainAtlasLlmService<I> {
    infra: Arc<I>,
}

impl<I> BrainAtlasLlmService<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

impl<E, I> BrainAtlasLlmService<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + LlmClient<Error = E>,
{
    /// Send a multi-turn chat with tool definitions, returning tool calls or final text
    pub async fn summarize_with_tools(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        chat_model_override: Option<&str>,
    ) -> Result<LlmResponse, ServiceError<E>> {
        let api_key = self
            .infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let chat_model = match chat_model_override {
            Some(m) => m.to_string(),
            None => self
                .infra
                .get("CHAT_MODEL")
                .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string()),
        };

        self.infra
            .summarize_with_tools(&api_key, &chat_model, messages, tools)
            .await
            .map_err(ServiceError::InfraError)
    }

    /// Generate search queries for a brain region
    pub async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, ServiceError<E>> {
        // Get API key and model from environment
        let api_key = self
            .infra
            .get("OPENROUTER_API_KEY")
            .map_err(ServiceError::InfraError)?;
        let chat_model = self
            .infra
            .get("CHAT_MODEL")
            .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string());

        self.infra
            .generate_queries(&api_key, &chat_model, region_name, count)
            .await
            .map_err(ServiceError::InfraError)
    }

    /// Run a single-turn structured-output chat: system + user, no tools.
    /// Returns the model's final text. Does not handle tool calls (rejects them as an error).
    async fn structured_chat(
        &self,
        system_prompt: &str,
        user_content: &str,
        chat_model_override: Option<&str>,
    ) -> Result<String, ServiceError<E>> {
        let messages = vec![
            serde_json::json!({"role": "system", "content": system_prompt}),
            serde_json::json!({"role": "user",   "content": user_content}),
        ];
        match self
            .summarize_with_tools(&messages, &[], chat_model_override)
            .await?
        {
            LlmResponse::Final(text) => Ok(text),
            LlmResponse::ToolCalls(_) => {
                warn!("structured_chat received unexpected tool calls; treating as empty response");
                Err(ServiceError::Other(
                    "LLM returned tool calls instead of final text for structured prompt"
                        .to_string(),
                ))
            }
        }
    }

    /// Extract atomic claims from a summary. Single LLM call returning structured JSON.
    pub async fn extract_claims(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model_override: Option<&str>,
    ) -> Result<ClaimsResponse, ServiceError<E>> {
        let system = EXTRACT_CLAIMS_SYSTEM.replace("{{REGION_NAME}}", region_name);
        let user = format!(
            "Brain region: {}\n\nSummary:\n\n{}",
            region_name, summary_text
        );
        let raw = self
            .structured_chat(&system, &user, chat_model_override)
            .await?;
        parse_json_loose::<ClaimsResponse>(&raw)
            .map_err(|e| ServiceError::Other(format!("extract_claims parse error: {e}")))
    }

    /// Judge whether a single claim is grounded in the supplied evidence chunks.
    pub async fn judge_groundedness(
        &self,
        claim_text: &str,
        evidence_chunks: &[String],
        chat_model_override: Option<&str>,
    ) -> Result<GroundednessVerdict, ServiceError<E>> {
        let mut user = String::new();
        user.push_str("Claim:\n");
        user.push_str(claim_text);
        user.push_str("\n\nEvidence chunks:\n");
        for (idx, chunk) in evidence_chunks.iter().enumerate() {
            user.push_str(&format!("\n[{}] {}\n", idx + 1, chunk));
        }
        if evidence_chunks.is_empty() {
            user.push_str("\n(no evidence chunks)\n");
        }
        let raw = self
            .structured_chat(JUDGE_GROUNDEDNESS_SYSTEM, &user, chat_model_override)
            .await?;
        parse_json_loose::<GroundednessVerdict>(&raw)
            .map_err(|e| ServiceError::Other(format!("judge_groundedness parse error: {e}")))
    }

    /// Score the summary against the fixed five-criterion rubric. Single LLM call.
    pub async fn judge_rubric(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model_override: Option<&str>,
    ) -> Result<RubricScores, ServiceError<E>> {
        let system = JUDGE_RUBRIC_SYSTEM.replace("{{REGION_NAME}}", region_name);
        let user = format!(
            "Brain region: {}\n\nSummary:\n\n{}",
            region_name, summary_text
        );
        let raw = self
            .structured_chat(&system, &user, chat_model_override)
            .await?;
        parse_json_loose::<RubricScores>(&raw)
            .map_err(|e| ServiceError::Other(format!("judge_rubric parse error: {e}")))
    }
}

// Prompt templates loaded at compile time. Live in the `app` crate per the
// project convention; we reference them via a relative path so the service
// layer can use them without a runtime file dependency.
const EXTRACT_CLAIMS_SYSTEM: &str =
    include_str!("../../app/prompts/extract_claims_system.md");
const JUDGE_GROUNDEDNESS_SYSTEM: &str =
    include_str!("../../app/prompts/judge_groundedness_system.md");
const JUDGE_RUBRIC_SYSTEM: &str =
    include_str!("../../app/prompts/judge_rubric_system.md");

/// Parse a JSON payload that may be wrapped in markdown fences or
/// surrounded by stray prose. Strips fences and falls back to the
/// outermost `{...}` substring before invoking serde.
fn parse_json_loose<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, serde_json::Error> {
    let trimmed = raw.trim();

    // Strip ```json ... ``` or ``` ... ``` fences if present.
    let stripped = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.trim_start().trim_end_matches("```").trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.trim_start().trim_end_matches("```").trim()
    } else {
        trimmed
    };

    if let Ok(v) = serde_json::from_str::<T>(stripped) {
        return Ok(v);
    }

    // Fallback: extract the outermost balanced { ... } region.
    if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}'))
        && start < end
    {
        return serde_json::from_str::<T>(&stripped[start..=end]);
    }

    // Force the original parse error to propagate.
    serde_json::from_str::<T>(stripped)
}
