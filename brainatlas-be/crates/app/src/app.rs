use crate::{AppError, Services};
use domain::{
    BrainRegionEntry, ChunkSource, LlmResponse, NewEmbedding, NewRegionSummary, RegionMapping,
    RetrievalFallbackPolicy, RetrievalScope, SearchEmbeddingsArgs, SimilarChunk, UsageContext,
    compute_hash, rpc_types::PaperMetadata,
};
use futures::future::join_all;
use schemars::schema_for;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

const MAX_TOOL_CALL_ITERATIONS: usize = 5;

// Load prompt templates at compile time
const RAG_SUMMARIZE_SYSTEM_TEMPLATE: &str = include_str!("../prompts/rag_summarize_system.md");
const RAG_SUMMARIZE_USER_TEMPLATE: &str = include_str!("../prompts/rag_summarize_user.md");
const KNOWLEDGE_SUMMARIZE_SYSTEM_TEMPLATE: &str =
    include_str!("../prompts/knowledge_summarize_system.md");

#[derive(Debug, Clone, Serialize)]
struct RegionIdentityContext {
    region_id: i32,
    name: String,
    acronym: Option<String>,
    parent_region_id: Option<i32>,
    parent_acronym: Option<String>,
    structure_order: Option<i32>,
    ontology_extension: Option<String>,
}

impl From<&RegionMapping> for RegionIdentityContext {
    fn from(region: &RegionMapping) -> Self {
        Self {
            region_id: region.region_id,
            name: region.name.clone(),
            acronym: region.acronym.clone(),
            parent_region_id: region.parent_region_id,
            parent_acronym: region.parent_acronym.clone(),
            structure_order: region.structure_order,
            ontology_extension: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct SearchEmbeddingsToolResult {
    target_region: RegionIdentityContext,
    retrieval_scope: RetrievalScope,
    results: Vec<SimilarChunk>,
}

fn render_region_context_block(region: &RegionMapping) -> String {
    let identity = RegionIdentityContext::from(region);
    let identity_json =
        serde_json::to_string_pretty(&identity).unwrap_or_else(|_| "{}".to_string());
    format!(
        "**Region identity metadata (authoritative):**\n```json\n{}\n```\nTreat `ontology_extension` as reserved for future ontology-backed enrichment; if it is null, do not infer missing type metadata.",
        identity_json
    )
}

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

    #[allow(clippy::too_many_arguments)]
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
        let correlation_id = correlation_id.unwrap_or_else(|| format!("batch:{batch_id}"));
        let base_ctx = UsageContext::default()
            .with_correlation(Some(correlation_id.clone()))
            .with_region(Some(region.region_id))
            .with_batch(Some(batch_id));

        // Build a map: s3_key -> metadata for quick lookup
        let metadata_map: HashMap<String, &PaperMetadata> = paper_metadata
            .iter()
            .map(|m| (m.s3_key.clone(), m))
            .collect();

        tracing::info!(
            region = %region.name,
            region_id = region.region_id,
            batch_id = %batch_id,
            s3_keys = s3_keys.len(),
            skip_summarization,
            "process_region: starting"
        );

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
            tracing::info!(
                region = %region.name,
                region_id = region.region_id,
                batch_id = %batch_id,
                existing_summary_id = %existing.summary_id,
                content_hash = %content_hash,
                "process_region: content hash matched — reusing existing summary, no LLM call"
            );
            return Ok(existing.summary_id);
        }

        tracing::info!(
            region = %region.name,
            region_id = region.region_id,
            batch_id = %batch_id,
            chunks = all_chunks.len(),
            "process_region: new content, proceeding to embed + summarize"
        );

        // 4. Generate embeddings for all chunks in parallel
        let embedding_futures: Vec<_> = all_chunks
            .iter()
            .map(|chunk| {
                self.services
                    .generate_embedding(chunk, embedding_model_ref, base_ctx.clone())
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
            let retrieval_scope = RetrievalScope::current_summary(region.region_id, summary_id)
                .with_fallback_policy(RetrievalFallbackPolicy::ActiveSummary);

            // 7. RAG summarization loop
            let summary_text = self
                .rag_summarize(
                    &region,
                    retrieval_scope,
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

    /// Knowledge-only path: used when NCBI search returns zero papers for a
    /// region. Generates a structured summary purely from the LLM's general
    /// / textbook knowledge (no retrieved chunks, no embeddings, no citations)
    /// so that every region in `region_mapping` ends up with at least one
    /// entry in `region_summary`.
    pub async fn process_region_no_papers(
        &self,
        uuid: Uuid,
        batch_id: Uuid,
        chat_model: Option<String>,
        correlation_id: Option<String>,
    ) -> Result<Uuid, AppError<E>> {
        let region = self.get_region_by_uuid(uuid).await?;

        let correlation_id = correlation_id.unwrap_or_else(|| format!("batch:{batch_id}"));
        let ctx = UsageContext::default()
            .with_correlation(Some(correlation_id.clone()))
            .with_region(Some(region.region_id))
            .with_batch(Some(batch_id))
            .with_caller_tag("knowledge_summarize");

        // Dedup on a stable per-region key so retries don't multiply rows.
        let content_hash = format!("knowledge-only:{}", region.region_id);

        // If a prior knowledge-only summary for this region exists with the
        // same content hash, return it rather than regenerating. This mirrors
        // the check in `process_region`.
        if let Some(existing) = self
            .services
            .check_content_hash(region.region_id, &content_hash)
            .await
            .map_err(AppError::ServiceError)?
        {
            tracing::info!(
                region = %region.name,
                region_id = region.region_id,
                batch_id = %batch_id,
                existing_summary_id = %existing.summary_id,
                "process_region_no_papers: knowledge-only hash matched — reusing existing summary, no LLM call"
            );
            return Ok(existing.summary_id);
        }

        // Build the system+user message pair. No tools — the LLM must return
        // a single final text response.
        let region_context_block = render_region_context_block(&region);
        let system_prompt = KNOWLEDGE_SUMMARIZE_SYSTEM_TEMPLATE
            .replace("{{REGION_NAME}}", &region.name)
            .replace("{{REGION_CONTEXT_BLOCK}}", &region_context_block);
        let user_prompt = format!("Please provide the structured summary for {}.", region.name);
        let messages: Vec<serde_json::Value> = vec![
            serde_json::json!({ "role": "system", "content": system_prompt }),
            serde_json::json!({ "role": "user", "content": user_prompt }),
        ];

        // Insert placeholder summary first so the usage rows (which reference
        // summary_id via ctx) have a valid FK. Empty embeddings — the service
        // layer must tolerate an empty slice.
        let new_summary = NewRegionSummary {
            region_id: region.region_id,
            name: region.name.clone(),
            acronym: region.acronym.clone(),
            summary: String::new(), // filled in via update_summary_text below
            content_hash,
            batch_id,
        };

        let summary_id = self
            .services
            .insert_summary_with_embeddings(new_summary, Vec::new())
            .await
            .map_err(AppError::ServiceError)?;

        let ctx = ctx.with_summary(Some(summary_id));

        info!(
            region = %region.name,
            region_id = region.region_id,
            summary_id = %summary_id,
            "knowledge-only summarization starting (no sources)"
        );

        let response = self
            .services
            .summarize_with_tools(
                &messages,
                &[], // no tools — forces a Final response
                chat_model.as_deref(),
                ctx,
            )
            .await
            .map_err(AppError::ServiceError)?;

        let summary_text = match response {
            LlmResponse::Final(text) => text,
            LlmResponse::ToolCalls(_) => {
                error!(
                    region = %region.name,
                    "knowledge-only summarization returned tool calls despite no tools being offered"
                );
                return Err(AppError::UnexpectedToolCall);
            }
        };

        info!(
            region = %region.name,
            summary_id = %summary_id,
            chars = summary_text.len(),
            "knowledge-only summary generated"
        );

        self.services
            .update_summary_text(summary_id, &summary_text)
            .await
            .map_err(AppError::ServiceError)?;

        Ok(summary_id)
    }

    /// RAG loop: LLM uses search_embeddings tool to retrieve context, then synthesizes a summary.
    async fn rag_summarize(
        &self,
        region: &RegionMapping,
        retrieval_scope: RetrievalScope,
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
        let region_context_block = render_region_context_block(region);
        let system_prompt = RAG_SUMMARIZE_SYSTEM_TEMPLATE
            .replace("{{REGION_NAME}}", &region.name)
            .replace("{{REGION_CONTEXT_BLOCK}}", &region_context_block);
        let user_prompt = RAG_SUMMARIZE_USER_TEMPLATE.replace("{{REGION_NAME}}", &region.name);

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
                region.name
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

                        let requested_fallback_policy = args
                            .fallback_policy
                            .unwrap_or(retrieval_scope.fallback_policy);
                        let active_fallback_requested =
                            requested_fallback_policy == RetrievalFallbackPolicy::ActiveSummary;
                        let effective_scope = RetrievalScope {
                            fallback_policy: requested_fallback_policy,
                            ..retrieval_scope.clone()
                        };

                        info!(
                            "Executing search_embeddings(query='{}', top_k={}, summary_scope={}, fallback={})",
                            args.query,
                            args.top_k,
                            effective_scope.summary_id,
                            if active_fallback_requested {
                                "active_summary"
                            } else {
                                "none"
                            }
                        );

                        // Generate embedding for the query
                        let query_embedding = self
                            .services
                            .generate_embedding(&args.query, embedding_model, ctx.clone())
                            .await
                            .map_err(AppError::ServiceError)?;

                        let similar_chunks = self
                            .services
                            .search_similar(query_embedding, effective_scope.clone(), args.top_k)
                            .await
                            .map_err(AppError::ServiceError)?;

                        info!(
                            "Found {} similar chunks for query '{}' within summary {} (fallback={})",
                            similar_chunks.len(),
                            args.query,
                            effective_scope.summary_id,
                            if active_fallback_requested {
                                "active_summary"
                            } else {
                                "none"
                            }
                        );

                        // Serialize results and add as tool response
                        let tool_result = SearchEmbeddingsToolResult {
                            target_region: RegionIdentityContext::from(region),
                            retrieval_scope: effective_scope,
                            results: similar_chunks,
                        };
                        let result_content =
                            serde_json::to_string(&tool_result).unwrap_or_default();

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
            MAX_TOOL_CALL_ITERATIONS, region.name
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
        correlation_id: Option<String>,
    ) -> Result<domain::GroundednessVerdict, AppError<E>> {
        let ctx = UsageContext::default().with_correlation(correlation_id);
        self.services
            .judge_citation(claim_text, sentence_context, chunk_text, chat_model, ctx)
            .await
            .map_err(AppError::ServiceError)
    }
}

#[cfg(test)]
mod tests {
    //! Orchestration-level tests for `BrainAtlasApp`.
    //!
    //! These exercise the code paths in `app.rs` itself; the services layer
    //! is stubbed with a hand-rolled `FakeServices` (recording pattern, no
    //! mockall) that implements every sub-trait of the `Services` umbrella.
    use super::*;
    use crate::services::{
        BrainRegionInfo, Chunker, EmbeddingService, ListBrainRegions, LlmService, S3Storage,
        UsageQuery, VectorDatabase,
    };
    use domain::{
        BrainRegionEntry, ChunkSource, ClaimsResponse, ExistingSummary, GroundednessVerdict,
        LlmResponse, NewEmbedding, NewRegionSummary, RegionMapping, RetrievalFallbackPolicy,
        RetrievalScope, RubricScores, SimilarChunk, ToolCall, UsageAggregate, UsageAggregateFilter,
        rpc_types::PaperMetadata,
    };
    use std::sync::Mutex;

    #[derive(Debug, thiserror::Error)]
    #[error("fake service error: {0}")]
    struct FakeErr(&'static str);

    /// FIFO-consumed canned responses for `summarize_with_tools`.
    #[allow(dead_code)]
    enum CannedSummarize {
        Ok(LlmResponse),
        Err(&'static str),
    }

    /// Records every method call for post-hoc assertion.
    #[derive(Default)]
    struct Calls {
        downloads: Vec<String>,
        chunked: usize,
        embeddings_generated: Vec<String>,
        content_hashes_checked: Vec<(i32, String)>,
        inserted_summary: Option<(NewRegionSummary, Vec<NewEmbedding>)>,
        summarize_calls: usize,
        summary_text_updates: Vec<(Uuid, String)>,
        searches: Vec<(RetrievalScope, usize)>,
        generate_queries_calls: Vec<(String, u32)>,
    }

    struct FakeServices {
        regions: Vec<RegionMapping>,
        downloads: std::collections::HashMap<String, String>,
        chunk_behaviour: ChunkBehaviour,
        summarize_queue: Mutex<Vec<CannedSummarize>>,
        /// Map content_hash -> existing summary (dedup).
        existing_by_hash: std::collections::HashMap<String, ExistingSummary>,
        calls: Mutex<Calls>,
        /// When Some, `search` returns this error.
        search_error: Option<&'static str>,
        /// When Some, `download` returns this error for the first call.
        download_error_on_first: bool,
        /// Pre-seeded `insert_summary_with_embeddings` summary_id.
        insert_summary_id: Uuid,
        /// Last-resort error toggles.
        fail_insert_summary: bool,
        fail_update_summary: bool,
        fail_generate_embedding: bool,
    }

    #[allow(dead_code)]
    enum ChunkBehaviour {
        /// For each input text, split into N equal-ish chunks by `chunk_size`.
        ByChunkSize,
        /// Return a fixed set of chunks regardless of input.
        Fixed(Vec<String>),
    }

    impl FakeServices {
        fn new() -> Self {
            Self {
                regions: vec![],
                downloads: std::collections::HashMap::new(),
                chunk_behaviour: ChunkBehaviour::ByChunkSize,
                summarize_queue: Mutex::new(vec![]),
                existing_by_hash: std::collections::HashMap::new(),
                calls: Mutex::new(Calls::default()),
                search_error: None,
                download_error_on_first: false,
                insert_summary_id: Uuid::new_v4(),
                fail_insert_summary: false,
                fail_update_summary: false,
                fail_generate_embedding: false,
            }
        }

        fn with_region(mut self, r: RegionMapping) -> Self {
            self.regions.push(r);
            self
        }

        fn with_download(mut self, key: &str, content: &str) -> Self {
            self.downloads.insert(key.to_string(), content.to_string());
            self
        }

        fn enqueue_summarize_ok(self, resp: LlmResponse) -> Self {
            self.summarize_queue
                .lock()
                .unwrap()
                .push(CannedSummarize::Ok(resp));
            self
        }

        #[allow(dead_code)]
        fn enqueue_summarize_err(self, msg: &'static str) -> Self {
            self.summarize_queue
                .lock()
                .unwrap()
                .push(CannedSummarize::Err(msg));
            self
        }
    }

    #[async_trait::async_trait]
    impl ListBrainRegions for FakeServices {
        type Error = FakeErr;
        async fn list(&self) -> Result<Vec<RegionMapping>, Self::Error> {
            Ok(self.regions.clone())
        }
    }

    #[async_trait::async_trait]
    impl BrainRegionInfo for FakeServices {
        type Error = FakeErr;
        async fn search(&self, _id: Uuid) -> Result<Vec<BrainRegionEntry>, Self::Error> {
            if let Some(msg) = self.search_error {
                return Err(FakeErr(msg));
            }
            Ok(vec![])
        }
    }

    impl Chunker for FakeServices {
        fn chunk(&self, text: &str, chunk_size: usize, _overlap: usize) -> Vec<String> {
            let mut calls = self.calls.lock().unwrap();
            calls.chunked += 1;
            drop(calls);
            match &self.chunk_behaviour {
                ChunkBehaviour::Fixed(v) => v.clone(),
                ChunkBehaviour::ByChunkSize => {
                    if text.is_empty() {
                        return vec![];
                    }
                    let step = chunk_size.max(1);
                    text.as_bytes()
                        .chunks(step)
                        .map(|b| String::from_utf8_lossy(b).to_string())
                        .collect()
                }
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmService for FakeServices {
        type Error = FakeErr;

        async fn summarize_with_tools(
            &self,
            _messages: &[serde_json::Value],
            _tools: &[serde_json::Value],
            _chat_model: Option<&str>,
            _ctx: UsageContext,
        ) -> Result<LlmResponse, Self::Error> {
            self.calls.lock().unwrap().summarize_calls += 1;
            let mut q = self.summarize_queue.lock().unwrap();
            if q.is_empty() {
                // Default to an infinite tool-call loop trigger so we can
                // test MAX_TOOL_CALL_ITERATIONS without pre-seeding five
                // entries.
                return Ok(LlmResponse::ToolCalls(vec![ToolCall {
                    id: "auto".to_string(),
                    name: "search_embeddings".to_string(),
                    arguments: r#"{"query":"x","top_k":1}"#.to_string(),
                }]));
            }
            match q.remove(0) {
                CannedSummarize::Ok(r) => Ok(r),
                CannedSummarize::Err(m) => Err(FakeErr(m)),
            }
        }

        async fn generate_queries(
            &self,
            region_name: &str,
            count: u32,
            _ctx: UsageContext,
        ) -> Result<Vec<String>, Self::Error> {
            self.calls
                .lock()
                .unwrap()
                .generate_queries_calls
                .push((region_name.to_string(), count));
            Ok((0..count).map(|i| format!("{region_name} q{i}")).collect())
        }

        async fn extract_claims(
            &self,
            _summary_text: &str,
            _region_name: &str,
            _chat_model: Option<&str>,
            _ctx: UsageContext,
        ) -> Result<ClaimsResponse, Self::Error> {
            Ok(ClaimsResponse { claims: vec![] })
        }

        async fn judge_groundedness(
            &self,
            _claim_text: &str,
            _evidence_chunks: &[String],
            _chat_model: Option<&str>,
            _ctx: UsageContext,
        ) -> Result<GroundednessVerdict, Self::Error> {
            Ok(GroundednessVerdict {
                verdict: domain::GroundednessLabel::Unsupported,
                confidence: 0.5,
                supporting_chunks: vec![],
                rationale: String::new(),
            })
        }

        async fn judge_rubric(
            &self,
            _summary_text: &str,
            _region_name: &str,
            _chat_model: Option<&str>,
            _ctx: UsageContext,
        ) -> Result<RubricScores, Self::Error> {
            let c = || domain::RubricCriterion {
                score: 3,
                rationale: String::new(),
            };
            Ok(RubricScores {
                relevance: c(),
                coherence: c(),
                specificity: c(),
                clinical_utility: c(),
                terminology: c(),
            })
        }

        async fn judge_citation(
            &self,
            _claim_text: &str,
            _sentence_context: &str,
            _chunk_text: &str,
            _chat_model: Option<&str>,
            _ctx: UsageContext,
        ) -> Result<GroundednessVerdict, Self::Error> {
            Ok(GroundednessVerdict {
                verdict: domain::GroundednessLabel::Supported,
                confidence: 1.0,
                supporting_chunks: vec![],
                rationale: String::new(),
            })
        }
    }

    #[async_trait::async_trait]
    impl EmbeddingService for FakeServices {
        type Error = FakeErr;
        async fn generate_embedding(
            &self,
            text: &str,
            _model_override: Option<&str>,
            _ctx: UsageContext,
        ) -> Result<Vec<f32>, Self::Error> {
            if self.fail_generate_embedding {
                return Err(FakeErr("embedding failed"));
            }
            self.calls
                .lock()
                .unwrap()
                .embeddings_generated
                .push(text.to_string());
            Ok(vec![0.1, 0.2, 0.3])
        }
    }

    #[async_trait::async_trait]
    impl S3Storage for FakeServices {
        type Error = FakeErr;
        async fn download(&self, key: &str) -> Result<String, Self::Error> {
            {
                let mut c = self.calls.lock().unwrap();
                if self.download_error_on_first && c.downloads.is_empty() {
                    c.downloads.push(key.to_string());
                    return Err(FakeErr("download failed"));
                }
                c.downloads.push(key.to_string());
            }
            self.downloads
                .get(key)
                .cloned()
                .ok_or(FakeErr("s3 key not found"))
        }
    }

    #[async_trait::async_trait]
    impl VectorDatabase for FakeServices {
        type Error = FakeErr;

        async fn check_content_hash(
            &self,
            region_id: i32,
            content_hash: &str,
        ) -> Result<Option<ExistingSummary>, Self::Error> {
            self.calls
                .lock()
                .unwrap()
                .content_hashes_checked
                .push((region_id, content_hash.to_string()));
            Ok(self.existing_by_hash.get(content_hash).cloned())
        }

        async fn insert_summary_with_embeddings(
            &self,
            summary: NewRegionSummary,
            embeddings: Vec<NewEmbedding>,
        ) -> Result<Uuid, Self::Error> {
            if self.fail_insert_summary {
                return Err(FakeErr("insert failed"));
            }
            self.calls.lock().unwrap().inserted_summary = Some((summary, embeddings));
            Ok(self.insert_summary_id)
        }

        async fn search_similar(
            &self,
            _query_embedding: Vec<f32>,
            retrieval_scope: RetrievalScope,
            top_k: usize,
        ) -> Result<Vec<SimilarChunk>, Self::Error> {
            self.calls
                .lock()
                .unwrap()
                .searches
                .push((retrieval_scope, top_k));
            Ok(vec![])
        }

        async fn update_summary_text(
            &self,
            summary_id: Uuid,
            summary_text: &str,
        ) -> Result<(), Self::Error> {
            if self.fail_update_summary {
                return Err(FakeErr("update failed"));
            }
            self.calls
                .lock()
                .unwrap()
                .summary_text_updates
                .push((summary_id, summary_text.to_string()));
            Ok(())
        }

        async fn get_chunk_source(
            &self,
            _chunk_id: Uuid,
        ) -> Result<Option<ChunkSource>, Self::Error> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl UsageQuery for FakeServices {
        type Error = FakeErr;
        async fn usage_aggregate(
            &self,
            _filter: UsageAggregateFilter,
        ) -> Result<UsageAggregate, Self::Error> {
            Ok(UsageAggregate::default())
        }
    }

    // ---------- Helpers ----------

    fn sample_region(region_id: i32) -> RegionMapping {
        RegionMapping::new(region_id, format!("Region {region_id}"))
    }

    fn paper_meta(s3_key: &str, pmc: Option<&str>) -> PaperMetadata {
        PaperMetadata {
            s3_key: s3_key.to_string(),
            pmc_id: pmc.map(|s| s.to_string()),
            uid: None,
            query: None,
        }
    }

    // ---------- Tests: list / search ----------

    #[tokio::test]
    async fn list_delegates_to_services() {
        let region = sample_region(1);
        let services = Arc::new(FakeServices::new().with_region(region.clone()));
        let app = BrainAtlasApp::new(services);
        let got = app.list().await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].region_id, 1);
    }

    #[tokio::test]
    async fn search_delegates_errors_through_app_error() {
        let mut svc = FakeServices::new();
        svc.search_error = Some("boom");
        let app = BrainAtlasApp::new(Arc::new(svc));
        let err = app.search(Uuid::new_v4()).await.expect_err("should err");
        match err {
            AppError::ServiceError(e) => assert_eq!(e.to_string(), "fake service error: boom"),
            other => panic!("expected ServiceError, got {:?}", other),
        }
    }

    // ---------- Tests: get_region_by_uuid / NotFound ----------

    #[tokio::test]
    async fn process_region_returns_not_found_when_uuid_missing() {
        let app = BrainAtlasApp::new(Arc::new(FakeServices::new()));
        let err = app
            .process_region(
                Uuid::new_v4(),
                Uuid::new_v4(),
                vec![],
                vec![],
                None,
                None,
                false,
                None,
            )
            .await
            .err()
            .unwrap();
        assert!(matches!(err, AppError::NotFound), "got {:?}", err);
    }

    // ---------- Tests: process_region happy paths ----------

    #[tokio::test]
    async fn process_region_skip_summarization_short_circuits() {
        let region = sample_region(7);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("paper1.pdf", "Hello world this is content for paper one.");
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        let batch_id = Uuid::new_v4();
        let summary_id = app
            .process_region(
                region_uuid,
                batch_id,
                vec!["paper1.pdf".to_string()],
                vec![paper_meta("paper1.pdf", Some("PMC1"))],
                None,
                None,
                /* skip_summarization */ true,
                Some("corr-xyz".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(summary_id, svc.insert_summary_id);

        let calls = svc.calls.lock().unwrap();
        // Should have called download + chunk once, inserted summary,
        // but NOT invoked summarize_with_tools or update_summary_text.
        assert_eq!(calls.downloads, vec!["paper1.pdf".to_string()]);
        assert_eq!(calls.summarize_calls, 0);
        assert!(calls.summary_text_updates.is_empty());
        assert!(calls.inserted_summary.is_some());
        let (ins_summary, embeds) = calls.inserted_summary.as_ref().unwrap();
        assert_eq!(ins_summary.batch_id, batch_id);
        assert!(ins_summary.summary.is_empty(), "placeholder when skipped");
        assert!(!embeds.is_empty(), "embeddings must be generated");
        // Source metadata wired through: first embedding has pmc_id = PMC1.
        assert_eq!(embeds[0].source_pmc_id.as_deref(), Some("PMC1"));
        assert_eq!(embeds[0].source_s3_key.as_deref(), Some("paper1.pdf"));
    }

    #[tokio::test]
    async fn process_region_runs_rag_loop_when_not_skipped() {
        let region = sample_region(9);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("a.txt", "alpha beta gamma")
            .enqueue_summarize_ok(LlmResponse::Final("final summary text".to_string()));
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        let summary_id = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["a.txt".to_string()],
                vec![paper_meta("a.txt", None)],
                Some("chat-model".to_string()),
                Some("embed-model".to_string()),
                false,
                None,
            )
            .await
            .unwrap();
        assert_eq!(summary_id, svc.insert_summary_id);
        let calls = svc.calls.lock().unwrap();
        assert_eq!(calls.summarize_calls, 1);
        assert_eq!(
            calls.summary_text_updates,
            vec![(svc.insert_summary_id, "final summary text".to_string())]
        );
    }

    #[tokio::test]
    async fn process_region_dedups_via_content_hash() {
        let region = sample_region(3);
        let region_uuid = region.id;
        let existing_id = Uuid::new_v4();
        let mut svc = FakeServices::new()
            .with_region(region)
            .with_download("same.txt", "identical content");
        // Pre-seed existing summary for the hash the app will compute.
        // Since the hash depends on the concatenation used internally,
        // just pre-insert all possible hashes via a wildcard-free key:
        // we compute it here using the same compute_hash helper.
        let expected_hash = domain::compute_hash("identical content\n\n---\n\n");
        svc.existing_by_hash.insert(
            expected_hash,
            ExistingSummary {
                summary_id: existing_id,
                summary: "old".to_string(),
            },
        );
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        let got = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["same.txt".to_string()],
                vec![paper_meta("same.txt", None)],
                None,
                None,
                false,
                None,
            )
            .await
            .unwrap();
        assert_eq!(got, existing_id, "dedup path returned existing id");
        let calls = svc.calls.lock().unwrap();
        // Should NOT have inserted a new summary nor invoked LLM.
        assert!(calls.inserted_summary.is_none());
        assert_eq!(calls.summarize_calls, 0);
        assert!(calls.embeddings_generated.is_empty());
    }

    // ---------- Tests: process_region error paths ----------

    #[tokio::test]
    async fn process_region_propagates_download_error() {
        let region = sample_region(1);
        let region_uuid = region.id;
        let mut svc = FakeServices::new().with_region(region);
        svc.download_error_on_first = true;
        let app = BrainAtlasApp::new(Arc::new(svc));
        let err = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["missing.txt".to_string()],
                vec![paper_meta("missing.txt", None)],
                None,
                None,
                true,
                None,
            )
            .await
            .err()
            .unwrap();
        assert!(matches!(err, AppError::ServiceError(_)));
    }

    #[tokio::test]
    async fn process_region_propagates_embedding_error() {
        let region = sample_region(1);
        let region_uuid = region.id;
        let mut svc = FakeServices::new()
            .with_region(region)
            .with_download("k.txt", "some text");
        svc.fail_generate_embedding = true;
        let app = BrainAtlasApp::new(Arc::new(svc));
        let err = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["k.txt".to_string()],
                vec![paper_meta("k.txt", None)],
                None,
                None,
                true,
                None,
            )
            .await
            .err()
            .unwrap();
        assert!(matches!(err, AppError::ServiceError(_)));
    }

    #[tokio::test]
    async fn process_region_propagates_insert_failure() {
        let region = sample_region(1);
        let region_uuid = region.id;
        let mut svc = FakeServices::new()
            .with_region(region)
            .with_download("k.txt", "some text");
        svc.fail_insert_summary = true;
        let app = BrainAtlasApp::new(Arc::new(svc));
        let err = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["k.txt".to_string()],
                vec![paper_meta("k.txt", None)],
                None,
                None,
                true,
                None,
            )
            .await
            .err()
            .unwrap();
        assert!(matches!(err, AppError::ServiceError(_)));
    }

    #[tokio::test]
    async fn process_region_propagates_update_summary_failure_when_not_skipped() {
        let region = sample_region(4);
        let region_uuid = region.id;
        let mut svc = FakeServices::new()
            .with_region(region)
            .with_download("k.txt", "some text")
            .enqueue_summarize_ok(LlmResponse::Final("ok".to_string()));
        svc.fail_update_summary = true;
        let app = BrainAtlasApp::new(Arc::new(svc));
        let err = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["k.txt".to_string()],
                vec![paper_meta("k.txt", None)],
                None,
                None,
                false,
                None,
            )
            .await
            .err()
            .unwrap();
        assert!(matches!(err, AppError::ServiceError(_)));
    }

    // ---------- Tests: RAG loop inside rag_summarize ----------

    #[tokio::test]
    async fn rag_loop_executes_tool_calls_then_returns_final() {
        let region = sample_region(2);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("p.txt", "content")
            // Iteration 1: tool call -> search_embeddings
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "call-1".to_string(),
                name: "search_embeddings".to_string(),
                arguments:
                    r#"{"query":"hippocampus","top_k":3,"fallback_policy":"active_summary"}"#
                        .to_string(),
            }]))
            // Iteration 2: final answer
            .enqueue_summarize_ok(LlmResponse::Final("done".to_string()));
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        let _ = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["p.txt".to_string()],
                vec![paper_meta("p.txt", None)],
                None,
                None,
                false,
                None,
            )
            .await
            .unwrap();
        let calls = svc.calls.lock().unwrap();
        assert_eq!(calls.summarize_calls, 2, "two LLM iterations");
        // search_similar was invoked once per tool call.
        assert_eq!(calls.searches.len(), 1);
        assert_eq!(calls.searches[0].1, 3, "top_k propagated");
        assert_eq!(calls.searches[0].0.region_id, 2);
        assert_eq!(
            calls.searches[0].0.fallback_policy,
            RetrievalFallbackPolicy::ActiveSummary
        );
    }

    #[tokio::test]
    async fn rag_loop_tolerates_unknown_tool_and_continues() {
        let region = sample_region(2);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("p.txt", "content")
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "call-1".to_string(),
                name: "unknown_tool".to_string(),
                arguments: "{}".to_string(),
            }]))
            .enqueue_summarize_ok(LlmResponse::Final("ok".to_string()));
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        app.process_region(
            region_uuid,
            Uuid::new_v4(),
            vec!["p.txt".to_string()],
            vec![paper_meta("p.txt", None)],
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();
        // search_similar must NOT have been called for the unknown tool.
        let calls = svc.calls.lock().unwrap();
        assert!(calls.searches.is_empty());
    }

    #[tokio::test]
    async fn rag_loop_tolerates_malformed_tool_arguments() {
        let region = sample_region(2);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("p.txt", "content")
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "call-bad".to_string(),
                name: "search_embeddings".to_string(),
                // Invalid JSON.
                arguments: "not json at all".to_string(),
            }]))
            .enqueue_summarize_ok(LlmResponse::Final("ok".to_string()));
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        app.process_region(
            region_uuid,
            Uuid::new_v4(),
            vec!["p.txt".to_string()],
            vec![paper_meta("p.txt", None)],
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();
        // search_similar was not called because parse failed.
        let calls = svc.calls.lock().unwrap();
        assert!(calls.searches.is_empty());
    }

    #[tokio::test]
    async fn rag_loop_returns_error_when_iterations_exceed_max() {
        let region = sample_region(5);
        let region_uuid = region.id;
        // Enqueue nothing — the fake defaults to emitting an infinite loop
        // of `search_embeddings` tool calls, which will exhaust the budget.
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("p.txt", "content");
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        let err = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["p.txt".to_string()],
                vec![paper_meta("p.txt", None)],
                None,
                None,
                false,
                None,
            )
            .await
            .err()
            .unwrap();
        match err {
            AppError::MaxToolCallsExceeded(n) => {
                assert_eq!(n, MAX_TOOL_CALL_ITERATIONS);
            }
            other => panic!("expected MaxToolCallsExceeded, got {other:?}"),
        }
        let calls = svc.calls.lock().unwrap();
        assert_eq!(calls.summarize_calls, MAX_TOOL_CALL_ITERATIONS);
    }

    // ---------- Tests: pass-through eval helpers ----------

    #[tokio::test]
    async fn generate_queries_delegates_and_builds_ctx() {
        let app = BrainAtlasApp::new(Arc::new(FakeServices::new()));
        let got = app
            .generate_queries("hippocampus", 3, Some("corr".to_string()), Some(42))
            .await
            .unwrap();
        assert_eq!(got.len(), 3);
        assert!(got[0].contains("hippocampus"));
    }

    #[tokio::test]
    async fn embed_and_eval_helpers_delegate_to_services() {
        let app = BrainAtlasApp::new(Arc::new(FakeServices::new()));
        let v = app.embed("hi", None, None).await.unwrap();
        assert_eq!(v.len(), 3);

        let claims = app
            .extract_claims("summary", "region", None, None)
            .await
            .unwrap();
        assert!(claims.claims.is_empty());

        let verdict = app
            .judge_groundedness("claim", &[], None, None)
            .await
            .unwrap();
        assert_eq!(verdict.verdict, domain::GroundednessLabel::Unsupported);

        let rubric = app
            .judge_rubric("summary", "region", None, None)
            .await
            .unwrap();
        // Default RubricScores -> all fields 0.
        let _ = rubric;

        let citation = app
            .judge_citation("claim", "sent", "chunk", None, None)
            .await
            .unwrap();
        assert_eq!(citation.verdict, domain::GroundednessLabel::Supported);
    }

    #[tokio::test]
    async fn get_chunk_source_returns_none_when_missing() {
        let app = BrainAtlasApp::new(Arc::new(FakeServices::new()));
        let got = app.get_chunk_source(Uuid::new_v4()).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn usage_aggregate_returns_default_and_is_plumbed() {
        let app = BrainAtlasApp::new(Arc::new(FakeServices::new()));
        let agg = app
            .usage_aggregate(UsageAggregateFilter::default())
            .await
            .unwrap();
        // Default filter -> default aggregate.
        let _ = agg;
    }

    // ---------- Tests: prompt rendering (Task 12) ----------

    /// `render_region_context_block` must produce a JSON block that
    /// includes every identity field from `RegionMapping`, including the
    /// reserved `ontology_extension` extension point.
    #[test]
    fn render_region_context_block_contains_all_identity_fields() {
        let region = RegionMapping::new(42, "Hippocampus".to_string())
            .with_acronym("HPC".to_string())
            .with_structure_order(100)
            .with_parent(41, Some("CTX".to_string()));

        let block = render_region_context_block(&region);

        assert!(block.contains("region_id"), "must include region_id");
        assert!(block.contains("\"Hippocampus\""), "must include name");
        assert!(block.contains("\"HPC\""), "must include acronym");
        assert!(block.contains("100"), "must include structure_order");
        assert!(block.contains("41"), "must include parent_region_id");
        assert!(block.contains("\"CTX\""), "must include parent_acronym");
        assert!(
            block.contains("ontology_extension"),
            "must include ontology_extension extension point"
        );
        assert!(
            block.contains("null"),
            "ontology_extension is null when not set via RegionMapping"
        );
    }

    /// Optional fields that are not set must render as JSON null so the
    /// model does not hallucinate values for them.
    #[test]
    fn render_region_context_block_uses_null_for_missing_optional_fields() {
        let region = RegionMapping::new(1, "Unknown region".to_string());

        let block = render_region_context_block(&region);

        assert!(
            block.contains("\"acronym\": null"),
            "missing acronym renders as null"
        );
        assert!(
            block.contains("\"parent_region_id\": null"),
            "missing parent_region_id renders as null"
        );
        assert!(
            block.contains("\"parent_acronym\": null"),
            "missing parent_acronym renders as null"
        );
        assert!(
            block.contains("\"structure_order\": null"),
            "missing structure_order renders as null"
        );
    }

    /// After both substitutions the RAG system prompt must not contain any
    /// unresolved `{{...}}` placeholder, and must embed the region name and
    /// the generated context block.
    #[test]
    fn rag_system_prompt_no_unresolved_placeholders() {
        let region = RegionMapping::new(7, "Taenia tecta, dorsal part".to_string())
            .with_acronym("TTd".to_string())
            .with_parent(777, Some("TT".to_string()));

        let block = render_region_context_block(&region);
        let prompt = RAG_SUMMARIZE_SYSTEM_TEMPLATE
            .replace("{{REGION_NAME}}", &region.name)
            .replace("{{REGION_CONTEXT_BLOCK}}", &block);

        assert!(
            !prompt.contains("{{"),
            "all placeholders must be substituted in RAG system prompt"
        );
        assert!(
            prompt.contains("Taenia tecta, dorsal part"),
            "region name must appear in prompt"
        );
        assert!(
            prompt.contains("\"TTd\""),
            "acronym must appear via context block"
        );
    }

    /// After both substitutions the knowledge-only system prompt must not
    /// contain any unresolved `{{...}}` placeholder.
    #[test]
    fn knowledge_system_prompt_no_unresolved_placeholders() {
        let region = RegionMapping::new(8, "Cerebellum".to_string()).with_acronym("CB".to_string());

        let block = render_region_context_block(&region);
        let prompt = KNOWLEDGE_SUMMARIZE_SYSTEM_TEMPLATE
            .replace("{{REGION_NAME}}", &region.name)
            .replace("{{REGION_CONTEXT_BLOCK}}", &block);

        assert!(
            !prompt.contains("{{"),
            "all placeholders must be substituted in knowledge system prompt"
        );
        assert!(
            prompt.contains("Cerebellum"),
            "region name must appear in prompt"
        );
        assert!(
            prompt.contains("\"CB\""),
            "acronym must appear via context block"
        );
    }

    // ---------- Tests: retrieval scope contract (Task 4 app-layer) ----------

    /// The `summary_id` threaded into `search_similar` must be exactly the
    /// one returned by `insert_summary_with_embeddings`, not the nil
    /// placeholder or any other value.
    #[tokio::test]
    async fn retrieval_scope_carries_inserted_summary_id() {
        let region = sample_region(3);
        let region_uuid = region.id;
        let expected_summary_id = Uuid::new_v4();
        let mut svc = FakeServices::new()
            .with_region(region)
            .with_download("doc.txt", "content")
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "t".to_string(),
                name: "search_embeddings".to_string(),
                arguments: r#"{"query":"anatomy","top_k":1}"#.to_string(),
            }]))
            .enqueue_summarize_ok(LlmResponse::Final("final answer".to_string()));
        svc.insert_summary_id = expected_summary_id;
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        app.process_region(
            region_uuid,
            Uuid::new_v4(),
            vec!["doc.txt".to_string()],
            vec![paper_meta("doc.txt", None)],
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let calls = svc.calls.lock().unwrap();
        assert_eq!(
            calls.searches[0].0.summary_id, expected_summary_id,
            "retrieval scope must carry the summary_id from insert_summary_with_embeddings"
        );
        assert_eq!(calls.searches[0].0.region_id, 3, "region_id must match");
    }

    /// When the LLM does not supply `fallback_policy` in its tool args,
    /// the effective retrieval scope inherits `ActiveSummary` from the
    /// process-level default built in `process_region`.
    #[tokio::test]
    async fn retrieval_scope_default_fallback_is_active_summary() {
        let region = sample_region(9);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("doc.txt", "content")
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "t".to_string(),
                name: "search_embeddings".to_string(),
                // No fallback_policy field — should inherit the process default
                arguments: r#"{"query":"anatomy","top_k":5}"#.to_string(),
            }]))
            .enqueue_summarize_ok(LlmResponse::Final("done".to_string()));
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        app.process_region(
            region_uuid,
            Uuid::new_v4(),
            vec!["doc.txt".to_string()],
            vec![paper_meta("doc.txt", None)],
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let calls = svc.calls.lock().unwrap();
        assert_eq!(
            calls.searches[0].0.fallback_policy,
            RetrievalFallbackPolicy::ActiveSummary,
            "absent fallback_policy in LLM args must inherit ActiveSummary from process default"
        );
    }

    /// When the LLM explicitly requests `fallback_policy = "none"`, the
    /// retrieval scope must override the process-level ActiveSummary default
    /// and use None instead.
    #[tokio::test]
    async fn retrieval_scope_llm_can_override_fallback_to_none() {
        let region = sample_region(10);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("doc.txt", "content")
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "t".to_string(),
                name: "search_embeddings".to_string(),
                arguments: r#"{"query":"function","top_k":5,"fallback_policy":"none"}"#.to_string(),
            }]))
            .enqueue_summarize_ok(LlmResponse::Final("done".to_string()));
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        app.process_region(
            region_uuid,
            Uuid::new_v4(),
            vec!["doc.txt".to_string()],
            vec![paper_meta("doc.txt", None)],
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let calls = svc.calls.lock().unwrap();
        assert_eq!(
            calls.searches[0].0.fallback_policy,
            RetrievalFallbackPolicy::None,
            "fallback_policy='none' in LLM args must override the process-level ActiveSummary default"
        );
    }

    /// Multiple tool calls in a single RAG loop must each receive a scope
    /// that carries the same `summary_id` and `region_id`, regardless of
    /// which iteration they fall in.
    #[tokio::test]
    async fn retrieval_scope_is_consistent_across_multiple_tool_calls() {
        let region = sample_region(11);
        let region_uuid = region.id;
        let expected_summary_id = Uuid::new_v4();
        let mut svc = FakeServices::new()
            .with_region(region)
            .with_download("doc.txt", "content")
            // Iteration 1: two tool calls in one assistant turn
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![
                ToolCall {
                    id: "t1".to_string(),
                    name: "search_embeddings".to_string(),
                    arguments: r#"{"query":"anatomy","top_k":2}"#.to_string(),
                },
                ToolCall {
                    id: "t2".to_string(),
                    name: "search_embeddings".to_string(),
                    arguments: r#"{"query":"function","top_k":3}"#.to_string(),
                },
            ]))
            .enqueue_summarize_ok(LlmResponse::Final("done".to_string()));
        svc.insert_summary_id = expected_summary_id;
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        app.process_region(
            region_uuid,
            Uuid::new_v4(),
            vec!["doc.txt".to_string()],
            vec![paper_meta("doc.txt", None)],
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let calls = svc.calls.lock().unwrap();
        assert_eq!(
            calls.searches.len(),
            2,
            "two tool calls must yield two searches"
        );
        for (i, (scope, _)) in calls.searches.iter().enumerate() {
            assert_eq!(
                scope.summary_id, expected_summary_id,
                "search {i}: summary_id must be consistent"
            );
            assert_eq!(
                scope.region_id, 11,
                "search {i}: region_id must be consistent"
            );
        }
        assert_eq!(calls.searches[0].1, 2, "first call: top_k = 2");
        assert_eq!(calls.searches[1].1, 3, "second call: top_k = 3");
    }

    #[tokio::test]
    async fn process_region_default_correlation_id_is_batch_prefix() {
        // We can't easily read the correlation_id back from the FakeServices
        // (it is opaque inside UsageContext), but we can assert the call
        // succeeds when correlation_id is None. This at minimum exercises
        // the `.unwrap_or_else(|| format!("batch:..."))` branch.
        let region = sample_region(1);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("a.txt", "alpha");
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc);
        let _ = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["a.txt".to_string()],
                vec![paper_meta("a.txt", None)],
                None,
                None,
                true,
                /* correlation_id */ None,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn process_region_attributes_chunks_to_multiple_s3_keys() {
        let region = sample_region(1);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("a.txt", "alpha")
            .with_download("b.txt", "beta");
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        let _ = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["a.txt".to_string(), "b.txt".to_string()],
                vec![
                    paper_meta("a.txt", Some("PMC-A")),
                    paper_meta("b.txt", Some("PMC-B")),
                ],
                None,
                None,
                true,
                None,
            )
            .await
            .unwrap();
        let calls = svc.calls.lock().unwrap();
        let (_sum, embeds) = calls.inserted_summary.as_ref().unwrap();
        // Must cover both source keys: at least one embedding per key.
        let keys: std::collections::HashSet<_> = embeds
            .iter()
            .filter_map(|e| e.source_s3_key.clone())
            .collect();
        assert!(keys.contains("a.txt"));
        assert!(keys.contains("b.txt"));
    }
}
