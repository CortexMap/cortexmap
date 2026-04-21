use crate::{AppError, Services};
use domain::{
    BrainRegionEntry, ChunkSource, LlmResponse, NewEmbedding, NewRegionSummary, RegionMapping,
    SearchEmbeddingsArgs, UsageContext, compute_hash, rpc_types::PaperMetadata,
};
use futures::future::join_all;
use schemars::schema_for;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

const MAX_TOOL_CALL_ITERATIONS: usize = 5;

// Load prompt templates at compile time
const RAG_SUMMARIZE_SYSTEM_TEMPLATE: &str = include_str!("../prompts/rag_summarize_system.md");
const RAG_SUMMARIZE_USER_TEMPLATE: &str = include_str!("../prompts/rag_summarize_user.md");

pub struct BrainAtlasApp<S> {
    services: Arc<S>,
}

impl<E, S> BrainAtlasApp<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E>,
{
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }

    pub async fn list(&self) -> Result<Vec<RegionMapping>, AppError<E>> {
        self.services.list().await.map_err(AppError::ServiceError)
    }

    pub async fn search(&self, id: Uuid) -> Result<Vec<BrainRegionEntry>, AppError<E>> {
        self.services
            .search(id)
            .await
            .map_err(AppError::ServiceError)
    }

    async fn get_region_by_uuid(&self, uuid: Uuid) -> Result<RegionMapping, AppError<E>> {
        // Get the list of all regions and find the one with matching UUID
        let regions = self.services.list().await.map_err(AppError::ServiceError)?;

        regions
            .into_iter()
            .find(|r| r.id == uuid)
            .ok_or_else(|| AppError::NotFound)
    }

    pub async fn process_region(
        &self,
        uuid: Uuid,
        batch_id: Uuid,
        s3_keys: Vec<String>,
        paper_metadata: Vec<PaperMetadata>,
        chat_model: Option<String>,
        embedding_model: Option<String>,
        skip_summarization: bool,
        correlation_id: Option<String>,
    ) -> Result<Uuid, AppError<E>> {
        let region = self.get_region_by_uuid(uuid).await?;
        let embedding_model_ref = embedding_model.as_deref();

        // Correlation defaults to `batch:<batch_id>` if the caller did not
        // supply one. `UsageContext` carries the correlation/region/batch
        // linkage into every LLM/embedding call made while processing this
        // region.
        let correlation_id =
            correlation_id.unwrap_or_else(|| format!("batch:{batch_id}"));
        let base_ctx = UsageContext::default()
            .with_correlation(Some(correlation_id.clone()))
            .with_region(Some(region.region_id))
            .with_batch(Some(batch_id));

        // Build a map: s3_key -> metadata for quick lookup
        let metadata_map: HashMap<String, &PaperMetadata> = paper_metadata
            .iter()
            .map(|m| (m.s3_key.clone(), m))
            .collect();

        // 1. Download all S3 files and track which S3 key each chunk came from
        //    Also track character offsets within each file for source attribution
        let mut chunks_with_source: Vec<(String, usize, usize)> = Vec::new(); // (s3_key, start_idx, end_idx)
        let mut all_chunks = Vec::new();
        let mut chunk_char_offsets: Vec<(i32, i32)> = Vec::new(); // (char_start, char_end) within the source file
        let mut full_text = String::new();

        let chunk_size: usize = 1000;
        let chunk_overlap: usize = 200;

        for key in &s3_keys {
            let content = self
                .services
                .download(key)
                .await
                .map_err(AppError::ServiceError)?;

            let start_idx = all_chunks.len();
            let key_chunks = self.services.chunk(&content, chunk_size, chunk_overlap);

            // Compute character offsets for each chunk within this file
            let content_len = content.len();
            let step = chunk_size.saturating_sub(chunk_overlap).max(1);
            let mut offset = 0usize;
            for _ in 0..key_chunks.len() {
                let char_start = offset;
                let char_end = (offset + chunk_size).min(content_len);
                chunk_char_offsets.push((char_start as i32, char_end as i32));
                if char_end >= content_len {
                    break;
                }
                offset += step;
            }

            all_chunks.extend(key_chunks);
            let end_idx = all_chunks.len();
            chunks_with_source.push((key.clone(), start_idx, end_idx));

            full_text.push_str(&content);
            full_text.push_str("\n\n---\n\n");
        }

        // 2. Compute SHA-256 hash for deduplication
        let content_hash = compute_hash(&full_text);

        // 3. Check if we already processed this exact content
        if let Some(existing) = self
            .services
            .check_content_hash(region.region_id, &content_hash)
            .await
            .map_err(AppError::ServiceError)?
        {
            return Ok(existing.summary_id);
        }

        // 4. Generate embeddings for all chunks in parallel
        let embedding_futures: Vec<_> = all_chunks
            .iter()
            .map(|chunk| {
                self.services.generate_embedding(
                    chunk,
                    embedding_model_ref,
                    base_ctx.clone(),
                )
            })
            .collect();

        let embedding_results = join_all(embedding_futures).await;

        // 5. Build NewEmbedding structs with source metadata
        let new_embeddings: Vec<_> = embedding_results
            .into_iter()
            .enumerate()
            .map(|(idx, result)| {
                let embedding = result.map_err(AppError::ServiceError)?;

                // Find which S3 key this chunk belongs to
                let (s3_key, metadata) = chunks_with_source
                    .iter()
                    .find(|(_, start, end)| idx >= *start && idx < *end)
                    .map(|(key, _, _)| {
                        let meta = metadata_map.get(key);
                        (key.clone(), meta)
                    })
                    .unwrap_or_else(|| (String::new(), None));

                // Get character offsets for this chunk within its source file
                let (char_start, char_end) = chunk_char_offsets.get(idx).copied().unwrap_or((0, 0));

                Ok(NewEmbedding {
                    region_id: region.region_id,
                    summary_id: Uuid::nil(), // Placeholder - set by insert_summary_with_embeddings
                    chunk_index: idx as i32,
                    chunk_text: all_chunks[idx].clone(),
                    embedding,
                    source_s3_key: Some(s3_key),
                    source_pmc_id: metadata.and_then(|m| m.pmc_id.clone()),
                    source_uid: metadata.and_then(|m| m.uid.clone()),
                    source_query: metadata.and_then(|m| m.query.clone()),
                    source_char_start: Some(char_start),
                    source_char_end: Some(char_end),
                })
            })
            .collect::<Result<Vec<_>, AppError<E>>>()?;

        // 6. Insert placeholder summary + embeddings (embeddings are now searchable)
        let new_summary = NewRegionSummary {
            region_id: region.region_id,
            name: region.name.clone(),
            acronym: region.acronym.clone(),
            summary: String::new(), // Placeholder, updated after RAG loop (if not skipped)
            content_hash,
            batch_id,
        };

        let summary_id = self
            .services
            .insert_summary_with_embeddings(new_summary, new_embeddings)
            .await
            .map_err(AppError::ServiceError)?;

        // 7-8. RAG summarization (skipped when background pipeline is just growing the knowledge base)
        if skip_summarization {
            info!(
                region = %region.name,
                region_id = region.region_id,
                summary_id = %summary_id,
                chunks = all_chunks.len(),
                "Chunk+embed complete (summarization skipped)"
            );
        } else {
            // 7. RAG summarization loop
            let summary_text = self
                .rag_summarize(
                    &region.name,
                    region.region_id,
                    chat_model.as_deref(),
                    embedding_model_ref,
                    base_ctx.clone().with_summary(Some(summary_id)),
                )
                .await?;

            // 8. Update the summary record with the final text
            self.services
                .update_summary_text(summary_id, &summary_text)
                .await
                .map_err(AppError::ServiceError)?;
        }

        Ok(summary_id)
    }

    /// RAG loop: LLM uses search_embeddings tool to retrieve context, then synthesizes a summary.
    async fn rag_summarize(
        &self,
        region_name: &str,
        region_id: i32,
        chat_model: Option<&str>,
        embedding_model: Option<&str>,
        ctx: UsageContext,
    ) -> Result<String, AppError<E>> {
        // Generate JSON schema for SearchEmbeddingsArgs using schemars
        let schema = schema_for!(SearchEmbeddingsArgs);
        let parameters_schema = serde_json::to_value(&schema).unwrap();

        // Build the tool definition
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "search_embeddings",
                "description": "Search the vector database for chunks relevant to a query about this brain region's research papers",
                "parameters": parameters_schema
            }
        })];

        // Load and substitute templates
        let system_prompt = RAG_SUMMARIZE_SYSTEM_TEMPLATE.replace("{{REGION_NAME}}", region_name);
        let user_prompt = RAG_SUMMARIZE_USER_TEMPLATE.replace("{{REGION_NAME}}", region_name);

        // Start the conversation with the system prompt
        let mut messages: Vec<serde_json::Value> = vec![serde_json::json!({
            "role": "system",
            "content": system_prompt
        })];

        // Initial user message to kick off the conversation
        messages.push(serde_json::json!({
            "role": "user",
            "content": user_prompt
        }));

        for iteration in 0..MAX_TOOL_CALL_ITERATIONS {
            info!(
                "RAG summarization iteration {} for region '{}'",
                iteration + 1,
                region_name
            );

            let response = self
                .services
                .summarize_with_tools(&messages, &tools, chat_model, ctx.clone())
                .await
                .map_err(AppError::ServiceError)?;

            match response {
                LlmResponse::Final(text) => {
                    info!(
                        "LLM returned final summary ({} chars) after {} iteration(s)",
                        text.len(),
                        iteration + 1
                    );
                    return Ok(text);
                }
                LlmResponse::ToolCalls(tool_calls) => {
                    // Add the assistant's tool-call message to history
                    let tool_calls_json: Vec<serde_json::Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments
                                }
                            })
                        })
                        .collect();

                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "tool_calls": tool_calls_json
                    }));

                    // Execute each tool call
                    for tc in &tool_calls {
                        if tc.name != "search_embeddings" {
                            warn!("Unknown tool call: {}, returning error", tc.name);
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tc.id,
                                "content": format!("Error: unknown tool '{}'", tc.name)
                            }));
                            continue;
                        }

                        let args: SearchEmbeddingsArgs = match serde_json::from_str(&tc.arguments) {
                            Ok(a) => a,
                            Err(e) => {
                                error!("Failed to parse tool call arguments: {}", e);
                                messages.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tc.id,
                                    "content": format!("Error parsing arguments: {}", e)
                                }));
                                continue;
                            }
                        };

                        info!(
                            "Executing search_embeddings(query='{}', top_k={})",
                            args.query, args.top_k
                        );

                        // Generate embedding for the query
                        let query_embedding = self
                            .services
                            .generate_embedding(&args.query, embedding_model, ctx.clone())
                            .await
                            .map_err(AppError::ServiceError)?;

                        // Search for similar chunks
                        let similar_chunks = self
                            .services
                            .search_similar(query_embedding, region_id, args.top_k)
                            .await
                            .map_err(AppError::ServiceError)?;

                        info!(
                            "Found {} similar chunks for query '{}'",
                            similar_chunks.len(),
                            args.query
                        );

                        // Serialize results and add as tool response
                        let result_content =
                            serde_json::to_string(&similar_chunks).unwrap_or_default();

                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tc.id,
                            "content": result_content
                        }));
                    }
                }
            }
        }

        // If we exceeded max iterations, return an error
        error!(
            "RAG loop exceeded {} iterations for region '{}'",
            MAX_TOOL_CALL_ITERATIONS, region_name
        );
        Err(AppError::MaxToolCallsExceeded(MAX_TOOL_CALL_ITERATIONS))
    }

    /// Generate search queries for a brain region using LLM
    pub async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
        correlation_id: Option<String>,
        region_id: Option<i32>,
    ) -> Result<Vec<String>, AppError<E>> {
        let ctx = UsageContext::default()
            .with_correlation(correlation_id)
            .with_region(region_id);
        self.services
            .generate_queries(region_name, count, ctx)
            .await
            .map_err(AppError::ServiceError)
    }

    /// Resolve a chunk UUID to its full source details
    pub async fn get_chunk_source(
        &self,
        chunk_id: Uuid,
    ) -> Result<Option<ChunkSource>, AppError<E>> {
        self.services
            .get_chunk_source(chunk_id)
            .await
            .map_err(AppError::ServiceError)
    }

    // ---- Eval LLM helpers (stateless wrappers over the LlmService trait) ----

    /// Generate an embedding for a single text string.
    pub async fn embed(
        &self,
        text: &str,
        embedding_model: Option<&str>,
        correlation_id: Option<String>,
    ) -> Result<Vec<f32>, AppError<E>> {
        let ctx = UsageContext::default().with_correlation(correlation_id);
        self.services
            .generate_embedding(text, embedding_model, ctx)
            .await
            .map_err(AppError::ServiceError)
    }

    /// Extract atomic claims from a summary.
    pub async fn extract_claims(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model: Option<&str>,
        correlation_id: Option<String>,
    ) -> Result<domain::ClaimsResponse, AppError<E>> {
        let ctx = UsageContext::default().with_correlation(correlation_id);
        self.services
            .extract_claims(summary_text, region_name, chat_model, ctx)
            .await
            .map_err(AppError::ServiceError)
    }

    /// Judge a single claim against retrieved evidence chunks.
    pub async fn judge_groundedness(
        &self,
        claim_text: &str,
        evidence_chunks: &[String],
        chat_model: Option<&str>,
        correlation_id: Option<String>,
    ) -> Result<domain::GroundednessVerdict, AppError<E>> {
        let ctx = UsageContext::default().with_correlation(correlation_id);
        self.services
            .judge_groundedness(claim_text, evidence_chunks, chat_model, ctx)
            .await
            .map_err(AppError::ServiceError)
    }

    /// Score the summary against the fixed five-criterion rubric.
    pub async fn judge_rubric(
        &self,
        summary_text: &str,
        region_name: &str,
        chat_model: Option<&str>,
        correlation_id: Option<String>,
    ) -> Result<domain::RubricScores, AppError<E>> {
        let ctx = UsageContext::default().with_correlation(correlation_id);
        self.services
            .judge_rubric(summary_text, region_name, chat_model, ctx)
            .await
            .map_err(AppError::ServiceError)
    }

    /// Aggregate LLM usage rows matching the supplied filter. Powers the
    /// `GET /brainatlas-be/api/llm/usage` endpoint.
    pub async fn usage_aggregate(
        &self,
        filter: domain::UsageAggregateFilter,
    ) -> Result<domain::UsageAggregate, AppError<E>> {
        self.services
            .usage_aggregate(filter)
            .await
            .map_err(AppError::ServiceError)
    }

    /// Judge whether a single cited chunk actually supports the attached claim.
    pub async fn judge_citation(
        &self,
        claim_text: &str,
        sentence_context: &str,
        chunk_text: &str,
        chat_model: Option<&str>,
    ) -> Result<domain::GroundednessVerdict, AppError<E>> {
        self.services
            .judge_citation(claim_text, sentence_context, chunk_text, chat_model)
            .await
            .map_err(AppError::ServiceError)
    }
}
