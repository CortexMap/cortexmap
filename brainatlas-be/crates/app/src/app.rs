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

const MAX_TOOL_CALL_ITERATIONS: usize = 10;
/// Hard cap on consecutive off-target query rejections inside a single RAG
/// loop before we abort and fall back to knowledge-only generation. Prevents
/// the loop from spinning forever when the model cannot produce an
/// acceptable on-topic query (Phase 3, Task 11).
const MAX_CONSECUTIVE_QUERY_REJECTIONS: usize = 3;

// Load prompt templates at compile time
const RAG_SUMMARIZE_SYSTEM_TEMPLATE: &str = include_str!("../prompts/rag_summarize_system.md");
const RAG_SUMMARIZE_USER_TEMPLATE: &str = include_str!("../prompts/rag_summarize_user.md");
const KNOWLEDGE_SUMMARIZE_SYSTEM_TEMPLATE: &str =
    include_str!("../prompts/knowledge_summarize_system.md");
const RAG_SUMMARIZE_LAYER_LEAF_SYSTEM_TEMPLATE: &str =
    include_str!("../prompts/rag_summarize_layer_leaf_system.md");
const RAG_SUMMARIZE_TRACT_PATHWAY_SYSTEM_TEMPLATE: &str =
    include_str!("../prompts/rag_summarize_tract_pathway_system.md");

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

/// Region-type template classifier (Phase 4 — Tasks 12, 13, 14).
///
/// Some region categories produce structurally bad summaries when forced
/// into the default 5-section template:
///
/// - Cortical layer leaves (`name` matches `\Wlayer\s+\d+`) only have
///   layer-specific evidence in rare papers; default behaviour is to defer
///   to the parent area and add a small layer-specific addendum.
/// - Tracts / fissures / pathways are connectivity-only objects; the
///   "Function" and "Clinical" sections fabricate when forced.
/// - Everything else uses the default nucleus/cortical-area template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum RegionTemplate {
    Default,
    CorticalLayerLeaf,
    TractOrPathway,
}

impl RegionTemplate {
    fn classify(region: &RegionMapping) -> Self {
        let name_lower = region.name.to_lowercase();
        // Layer leaves: "layer 5", "layer 2/3", "layer 6a"
        if regex_lite_word_match(&name_lower, "layer ")
            && name_lower
                .split("layer ")
                .nth(1)
                .map(|after| after.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .unwrap_or(false)
        {
            return Self::CorticalLayerLeaf;
        }
        // Tracts, fissures, pathways, fasciculus, peduncle, commissure, capsule
        for needle in [
            "tract",
            "fissure",
            "fasciculus",
            "pathway",
            "peduncle",
            "commissure",
            "capsule",
        ] {
            if name_lower.contains(needle) {
                return Self::TractOrPathway;
            }
        }
        Self::Default
    }

    fn system_template(self) -> &'static str {
        match self {
            Self::Default => RAG_SUMMARIZE_SYSTEM_TEMPLATE,
            Self::CorticalLayerLeaf => RAG_SUMMARIZE_LAYER_LEAF_SYSTEM_TEMPLATE,
            Self::TractOrPathway => RAG_SUMMARIZE_TRACT_PATHWAY_SYSTEM_TEMPLATE,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::CorticalLayerLeaf => "cortical_layer_leaf",
            Self::TractOrPathway => "tract_or_pathway",
        }
    }
}

/// Cheap word-boundary substring check that does not pull in regex crate.
/// Returns true if `needle` appears in `haystack` at a word boundary on
/// the left edge (right edge is approximate — we only need to catch
/// "layer 5" in "layer 5b" and similar).
fn regex_lite_word_match(haystack: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(idx) = haystack[start..].find(needle) {
        let abs = start + idx;
        let left_ok = abs == 0
            || haystack[..abs]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_alphanumeric());
        if left_ok {
            return true;
        }
        start = abs + needle.len();
    }
    false
}

/// **Phase 1, Task 1 — region-mention chunk filter.**
///
/// A chunk is considered to mention the target region when its text
/// contains *any* of the following (case-insensitive substring for the
/// name, word-boundary match for short acronyms):
///
/// - The region's full name
/// - The region's acronym (acronyms ≥ 2 chars are tested with word
///   boundaries to avoid matching e.g. "TT" inside "TTL")
/// - The parent region's acronym
///
/// Acronyms shorter than 2 characters are ignored entirely — they
/// produce too many false positives across unrelated text.
///
/// This is a high-precision *textual* test. A chunk that semantically
/// describes the region but never names it will be dropped — that is the
/// intentional trade-off documented in the plan. The pre-embedding filter
/// has shown <1 % of fetched chunks actually mention their target region.
pub(crate) fn chunk_mentions_region(chunk_text: &str, region: &RegionMapping) -> bool {
    text_mentions_region(chunk_text, region)
}

/// **Phase 3, Task 9 — query-emit guard.**
///
/// Same matcher as `chunk_mentions_region`, applied to LLM-emitted search
/// queries to keep the embedded corpus from being polluted with chunks
/// retrieved for entirely off-target queries.
pub(crate) fn query_mentions_region(query: &str, region: &RegionMapping) -> bool {
    text_mentions_region(query, region)
}

fn text_mentions_region(text: &str, region: &RegionMapping) -> bool {
    let text_lower = text.to_lowercase();
    let name_lower = region.name.to_lowercase();
    if !name_lower.is_empty() && text_lower.contains(&name_lower) {
        return true;
    }
    if let Some(acro) = region.acronym.as_deref() {
        if acronym_matches(text, acro) {
            return true;
        }
    }
    if let Some(parent_acro) = region.parent_acronym.as_deref() {
        if acronym_matches(text, parent_acro) {
            return true;
        }
    }
    false
}

/// Word-boundary match for acronyms. Acronyms are case-sensitive (so
/// "ECT" is not matched by "ect" inside "affect"), and ignored when
/// shorter than 2 characters.
fn acronym_matches(text: &str, acronym: &str) -> bool {
    let acronym = acronym.trim();
    if acronym.len() < 2 {
        return false;
    }
    let bytes = text.as_bytes();
    let needle = acronym.as_bytes();
    if bytes.len() < needle.len() {
        return false;
    }
    for i in 0..=bytes.len() - needle.len() {
        if &bytes[i..i + needle.len()] != needle {
            continue;
        }
        let left_ok = i == 0 || !is_acronym_neighbor_byte(bytes[i - 1]);
        let right_idx = i + needle.len();
        let right_ok = right_idx == bytes.len() || !is_acronym_neighbor_byte(bytes[right_idx]);
        if left_ok && right_ok {
            return true;
        }
    }
    false
}

fn is_acronym_neighbor_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// **Phase 1, Task 4 — RAG loop outcome bundle.**
///
/// Returned from `rag_summarize` so the caller can attribute per-summary
/// telemetry (number of LLM-emitted queries, number rejected by the
/// region-mention guard, total tool-call iterations) on top of the
/// final summary text.
#[derive(Debug, Clone, Default)]
struct RagOutcome {
    text: String,
    queries_emitted: usize,
    queries_rejected: usize,
    iterations: usize,
}

/// **Phase 7, Task 20 — per-summary observability struct.**
///
/// One of these is emitted as a structured `info!` event at the end of
/// every `process_region` invocation that runs through the full pipeline.
/// Operators can filter logs by `summary_id` and reconstruct what the
/// pipeline did: how many chunks survived the filter, how many queries
/// the model emitted that were rejected, what template variant was used.
#[derive(Debug, Clone, Serialize)]
struct RagRunTelemetry {
    region_id: i32,
    summary_id: Uuid,
    batch_id: Uuid,
    template: &'static str,
    chunks_total: usize,
    chunks_kept: usize,
    chunks_dropped: usize,
    queries_emitted: usize,
    queries_rejected: usize,
    rag_iterations: usize,
    routed_to_knowledge_only: bool,
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

        // 3b. **Phase 1 — region-mention chunk filter (Tasks 1, 2, 3).**
        //
        // Drop any chunk whose text never names the target region. This
        // pre-embedding filter is the highest-impact lever in the plan:
        // <1 % of fetched chunks actually mention their target region, so
        // we burn embedding cost on noise and the RAG loop retrieves
        // off-topic chunks that drag groundedness to ~0.04.
        //
        // We keep filtering OFF when `skip_summarization` is true so that
        // the background corpus-growth pipeline still embeds everything.
        let chunks_total = all_chunks.len();
        let (kept_chunks, kept_offsets, kept_sources, chunks_dropped) = if skip_summarization {
            // Identity pass — no filter when we are only growing the corpus.
            let kept: Vec<(usize, String)> = all_chunks
                .iter()
                .enumerate()
                .map(|(i, c)| (i, c.clone()))
                .collect();
            let offsets: Vec<(i32, i32)> = chunk_char_offsets.clone();
            let sources: Vec<(String, Option<&PaperMetadata>)> = (0..all_chunks.len())
                .map(|idx| {
                    let (s3_key, metadata) = chunks_with_source
                        .iter()
                        .find(|(_, start, end)| idx >= *start && idx < *end)
                        .map(|(key, _, _)| (key.clone(), metadata_map.get(key.as_str()).copied()))
                        .unwrap_or_else(|| (String::new(), None));
                    (s3_key, metadata)
                })
                .collect();
            (kept, offsets, sources, 0usize)
        } else {
            let mut kept: Vec<(usize, String)> = Vec::with_capacity(all_chunks.len());
            let mut offsets: Vec<(i32, i32)> = Vec::with_capacity(all_chunks.len());
            let mut sources: Vec<(String, Option<&PaperMetadata>)> =
                Vec::with_capacity(all_chunks.len());
            let mut dropped = 0usize;
            for (idx, chunk) in all_chunks.iter().enumerate() {
                if !chunk_mentions_region(chunk, &region) {
                    dropped += 1;
                    continue;
                }
                let (s3_key, metadata) = chunks_with_source
                    .iter()
                    .find(|(_, start, end)| idx >= *start && idx < *end)
                    .map(|(key, _, _)| (key.clone(), metadata_map.get(key.as_str()).copied()))
                    .unwrap_or_else(|| (String::new(), None));
                let (cs, ce) = chunk_char_offsets.get(idx).copied().unwrap_or((0, 0));
                kept.push((idx, chunk.clone()));
                offsets.push((cs, ce));
                sources.push((s3_key, metadata));
            }
            (kept, offsets, sources, dropped)
        };

        let chunks_kept = kept_chunks.len();
        info!(
            region = %region.name,
            region_id = region.region_id,
            batch_id = %batch_id,
            chunks_total,
            chunks_kept,
            chunks_dropped,
            "process_region: region-mention chunk filter applied"
        );

        // **Phase 1, Task 3.** When the filter leaves zero chunks, the
        // region has no on-topic literature in the fetched corpus. The
        // honest output is knowledge-only abstention rather than an
        // evidence-claiming summary built on irrelevant chunks.
        if !skip_summarization && chunks_kept == 0 && chunks_total > 0 {
            warn!(
                region = %region.name,
                region_id = region.region_id,
                batch_id = %batch_id,
                chunks_total,
                "process_region: region-mention filter eliminated all chunks; routing to knowledge-only summary"
            );
            let summary_id = self
                .process_region_no_papers(uuid, batch_id, chat_model, Some(correlation_id))
                .await?;
            info!(
                region = %region.name,
                region_id = region.region_id,
                summary_id = %summary_id,
                template = "knowledge_only_after_filter",
                chunks_total,
                chunks_kept = 0,
                chunks_dropped,
                routed_to_knowledge_only = true,
                "rag_run_telemetry"
            );
            return Ok(summary_id);
        }

        tracing::info!(
            region = %region.name,
            region_id = region.region_id,
            batch_id = %batch_id,
            chunks = chunks_kept,
            "process_region: new content, proceeding to embed + summarize"
        );

        // 4. Generate embeddings for surviving chunks in parallel
        let embedding_futures: Vec<_> = kept_chunks
            .iter()
            .map(|(_, chunk)| {
                self.services
                    .generate_embedding(chunk, embedding_model_ref, base_ctx.clone())
            })
            .collect();

        let embedding_results = join_all(embedding_futures).await;

        // 5. Build NewEmbedding structs with source metadata
        let new_embeddings: Vec<_> = embedding_results
            .into_iter()
            .enumerate()
            .map(|(out_idx, result)| {
                let embedding = result.map_err(AppError::ServiceError)?;
                let (orig_idx, chunk_text) = &kept_chunks[out_idx];
                let (s3_key, metadata) = &kept_sources[out_idx];
                let (char_start, char_end) = kept_offsets[out_idx];

                Ok(NewEmbedding {
                    region_id: region.region_id,
                    summary_id: Uuid::nil(), // Placeholder - set by insert_summary_with_embeddings
                    chunk_index: *orig_idx as i32,
                    chunk_text: chunk_text.clone(),
                    embedding,
                    source_s3_key: Some(s3_key.clone()),
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
                chunks = chunks_kept,
                "Chunk+embed complete (summarization skipped)"
            );
        } else {
            let retrieval_scope = RetrievalScope::current_summary(region.region_id, summary_id)
                .with_fallback_policy(RetrievalFallbackPolicy::ActiveSummary);

            let template = RegionTemplate::classify(&region);

            // 7. RAG summarization loop
            let rag_outcome = self
                .rag_summarize(
                    &region,
                    template,
                    retrieval_scope,
                    chat_model.as_deref(),
                    embedding_model_ref,
                    base_ctx.clone().with_summary(Some(summary_id)),
                )
                .await?;

            // 8. Update the summary record with the final text
            self.services
                .update_summary_text(summary_id, &rag_outcome.text)
                .await
                .map_err(AppError::ServiceError)?;

            // **Phase 7, Task 20** — emit per-summary observability event.
            let telemetry = RagRunTelemetry {
                region_id: region.region_id,
                summary_id,
                batch_id,
                template: template.label(),
                chunks_total,
                chunks_kept,
                chunks_dropped,
                queries_emitted: rag_outcome.queries_emitted,
                queries_rejected: rag_outcome.queries_rejected,
                rag_iterations: rag_outcome.iterations,
                routed_to_knowledge_only: false,
            };
            info!(
                telemetry = %serde_json::to_string(&telemetry).unwrap_or_default(),
                region = %region.name,
                region_id = region.region_id,
                summary_id = %summary_id,
                template = telemetry.template,
                chunks_total,
                chunks_kept,
                chunks_dropped,
                queries_emitted = telemetry.queries_emitted,
                queries_rejected = telemetry.queries_rejected,
                rag_iterations = telemetry.rag_iterations,
                "rag_run_telemetry"
            );
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
    ///
    /// Returns a `RagOutcome` bundling the summary text and per-summary
    /// telemetry counters so the caller can emit structured observability
    /// events (Phase 7, Task 20).
    async fn rag_summarize(
        &self,
        region: &RegionMapping,
        template: RegionTemplate,
        retrieval_scope: RetrievalScope,
        chat_model: Option<&str>,
        embedding_model: Option<&str>,
        ctx: UsageContext,
    ) -> Result<RagOutcome, AppError<E>> {
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

        // **Phase 4 — region-type-aware system prompt selection.** The
        // template's `system_template()` is a full prompt string; we no
        // longer compose section instructions inline.
        let region_context_block = render_region_context_block(region);
        let acronym_for_prompt = region.acronym.as_deref().unwrap_or(&region.name);
        let system_prompt = template
            .system_template()
            .replace("{{REGION_NAME}}", &region.name)
            .replace("{{REGION_ACRONYM}}", acronym_for_prompt)
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

        // Outcome stats threaded back to caller for per-summary telemetry.
        let mut outcome = RagOutcome::default();

        for iteration in 0..MAX_TOOL_CALL_ITERATIONS {
            outcome.iterations = iteration + 1;
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
                        "LLM returned final summary ({} chars) after {} iteration(s); \
                         queries_rejected={}",
                        text.len(),
                        iteration + 1,
                        outcome.queries_rejected
                    );
                    return Ok(RagOutcome {
                        text,
                        queries_emitted: outcome.queries_emitted,
                        queries_rejected: outcome.queries_rejected,
                        iterations: iteration + 1,
                    });
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

                        outcome.queries_emitted += 1;

                        // Phase 3: validate the LLM's free-form query before
                        // spending an embedding call on it. If it fails the
                        // on-target test, return a structured rejection so the
                        // model can refine. Cap rejections per summary so a
                        // looping model can't burn the iteration budget.
                        if !query_mentions_region(&args.query, region) {
                            outcome.queries_rejected += 1;
                            if outcome.queries_rejected > MAX_CONSECUTIVE_QUERY_REJECTIONS {
                                warn!(
                                    "Rejected-query cap ({}) hit for region '{}'; \
                                     accepting subsequent queries to avoid loops",
                                    MAX_CONSECUTIVE_QUERY_REJECTIONS, region.name
                                );
                                // fall through and accept
                            } else {
                                warn!(
                                    "Rejecting off-target query for region '{}': '{}'",
                                    region.name, args.query
                                );
                                let rejection = serde_json::json!({
                                    "rejected": true,
                                    "reason": format!(
                                        "The query must explicitly reference the target region. \
                                         Include the region name (\"{}\") or its acronym (\"{}\") \
                                         in the search query.",
                                        region.name,
                                        region.acronym.as_deref().unwrap_or("")
                                    ),
                                    "rejected_query": args.query,
                                });
                                messages.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tc.id,
                                    "content": rejection.to_string()
                                }));
                                continue;
                            }
                        }

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
            "RAG loop exceeded {} iterations for region '{}' (queries_rejected={})",
            MAX_TOOL_CALL_ITERATIONS, region.name, outcome.queries_rejected
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
        acronym: Option<&str>,
        parent_name: Option<&str>,
        parent_acronym: Option<&str>,
    ) -> Result<Vec<String>, AppError<E>> {
        let ctx = UsageContext::default()
            .with_correlation(correlation_id)
            .with_region(region_id);
        self.services
            .generate_queries(region_name, count, acronym, parent_name, parent_acronym, ctx)
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
            _acronym: Option<&str>,
            _parent_name: Option<&str>,
            _parent_acronym: Option<&str>,
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
        // Region name kept short and content-friendly so test fixtures can
        // naturally embed the name in chunk text and search queries (the
        // Phase 1 region-mention filter and Phase 3 query-rejection guard
        // both require the region name to appear).
        RegionMapping::new(region_id, "Cortex".to_string())
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
            .with_download("p.txt", "Cortex chunk content for tests")
            // Iteration 1: tool call -> search_embeddings (query mentions Cortex to pass region-mention guard)
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "call-1".to_string(),
                name: "search_embeddings".to_string(),
                arguments:
                    r#"{"query":"Cortex anatomy","top_k":3,"fallback_policy":"active_summary"}"#
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
            .with_download("p.txt", "Cortex content")
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
            .with_download("p.txt", "Cortex content")
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
            .with_download("p.txt", "Cortex content");
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
            .generate_queries(
                "hippocampus",
                3,
                Some("corr".to_string()),
                Some(42),
                None,
                None,
                None,
            )
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

    /// After all substitutions the RAG system prompt must not contain any
    /// unresolved `{{...}}` placeholder, and must embed the region name,
    /// acronym, and the generated context block.
    #[test]
    fn rag_system_prompt_no_unresolved_placeholders() {
        let region = RegionMapping::new(7, "Taenia tecta, dorsal part".to_string())
            .with_acronym("TTd".to_string())
            .with_parent(777, Some("TT".to_string()));

        let block = render_region_context_block(&region);
        let acronym = region.acronym.as_deref().unwrap_or(&region.name);
        let prompt = RAG_SUMMARIZE_SYSTEM_TEMPLATE
            .replace("{{REGION_NAME}}", &region.name)
            .replace("{{REGION_ACRONYM}}", acronym)
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
        assert!(
            prompt.contains("`TTd`"),
            "acronym must appear via REGION_ACRONYM substitution in search-strategy section"
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
            .with_download("doc.txt", "Cortex content for region")
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "t".to_string(),
                name: "search_embeddings".to_string(),
                arguments: r#"{"query":"Cortex anatomy","top_k":1}"#.to_string(),
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
            .with_download("doc.txt", "Cortex content for region")
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "t".to_string(),
                name: "search_embeddings".to_string(),
                // No fallback_policy field — should inherit the process default
                arguments: r#"{"query":"Cortex anatomy","top_k":5}"#.to_string(),
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
            .with_download("doc.txt", "Cortex content for region")
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "t".to_string(),
                name: "search_embeddings".to_string(),
                arguments: r#"{"query":"Cortex function","top_k":5,"fallback_policy":"none"}"#.to_string(),
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
            .with_download("doc.txt", "Cortex content for region")
            // Iteration 1: two tool calls in one assistant turn
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![
                ToolCall {
                    id: "t1".to_string(),
                    name: "search_embeddings".to_string(),
                    arguments: r#"{"query":"Cortex anatomy","top_k":2}"#.to_string(),
                },
                ToolCall {
                    id: "t2".to_string(),
                    name: "search_embeddings".to_string(),
                    arguments: r#"{"query":"Cortex function","top_k":3}"#.to_string(),
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

    // ---------- Tests: Phase 1 — chunk filter ----------

    #[test]
    fn chunk_mentions_region_matches_full_name_case_insensitive() {
        let region = RegionMapping::new(7, "Hippocampus".to_string());
        assert!(chunk_mentions_region(
            "the HIPPOCAMPUS plays a role in memory",
            &region
        ));
        assert!(chunk_mentions_region(
            "studies of the hippocampus",
            &region
        ));
        assert!(!chunk_mentions_region(
            "the cortex projects to the thalamus",
            &region
        ));
    }

    #[test]
    fn chunk_mentions_region_matches_acronym_with_word_boundaries() {
        let mut region = RegionMapping::new(7, "Hippocampus".to_string());
        region.acronym = Some("HPC".to_string());
        assert!(chunk_mentions_region("lesions of HPC abolished", &region));
        // Substring inside another token should NOT match.
        assert!(!chunk_mentions_region(
            "the HPCSomething construct",
            &region
        ));
    }

    #[test]
    fn chunk_mentions_region_matches_parent_acronym() {
        let mut region = RegionMapping::new(7, "Layer 5".to_string());
        region.acronym = Some("L5".to_string());
        region.parent_acronym = Some("M1".to_string());
        // Chunk only mentions parent — still considered relevant.
        assert!(chunk_mentions_region("M1 cortex projections", &region));
        assert!(!chunk_mentions_region("unrelated content", &region));
    }

    #[test]
    fn chunk_mentions_region_ignores_acronyms_shorter_than_two_chars() {
        let mut region = RegionMapping::new(7, "Hypothetical".to_string());
        region.acronym = Some("X".to_string());
        // 'X' must not match — too short, would create false positives.
        assert!(!chunk_mentions_region("the X-ray showed changes", &region));
    }

    #[tokio::test]
    async fn process_region_filter_drops_unrelated_chunks_when_summarizing() {
        // Chunk text never mentions "Cortex" (the region's name) — must be
        // dropped by the Phase 1 filter, leaving zero chunks → routed to
        // knowledge-only path which uses no tools.
        let region = sample_region(99);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download(
                "off.txt",
                "this document only talks about thalamus and basal ganglia",
            )
            // Knowledge-only path: no tools, single Final response expected.
            .enqueue_summarize_ok(LlmResponse::Final("knowledge-only summary".to_string()));
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        let _summary_id = app
            .process_region(
                region_uuid,
                Uuid::new_v4(),
                vec!["off.txt".to_string()],
                vec![paper_meta("off.txt", None)],
                None,
                None,
                false,
                None,
            )
            .await
            .unwrap();
        let calls = svc.calls.lock().unwrap();
        // No embeddings should have been generated for the dropped chunks.
        assert_eq!(
            calls.embeddings_generated.len(),
            0,
            "dropped chunks must not be embedded"
        );
        // No vector search either — the knowledge-only path doesn't retrieve.
        assert!(
            calls.searches.is_empty(),
            "knowledge-only path issues no vector searches"
        );
    }

    #[tokio::test]
    async fn process_region_filter_keeps_chunks_that_mention_region() {
        let region = sample_region(99);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("good.txt", "Cortex anatomy is well described in this paper")
            .enqueue_summarize_ok(LlmResponse::Final("done".to_string()));
        let svc = Arc::new(svc);
        let app = BrainAtlasApp::new(svc.clone());
        app.process_region(
            region_uuid,
            Uuid::new_v4(),
            vec!["good.txt".to_string()],
            vec![paper_meta("good.txt", None)],
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();
        let calls = svc.calls.lock().unwrap();
        // At least one chunk survived → at least one embedding generated.
        assert!(
            calls.embeddings_generated.len() >= 1,
            "filter must keep on-topic chunks"
        );
    }

    // ---------- Tests: Phase 3 — query rejection ----------

    #[tokio::test]
    async fn rag_loop_rejects_off_target_query_and_continues() {
        // The model emits an off-target query first, gets rejected, then
        // emits an on-target one and finishes.
        let region = sample_region(2);
        let region_uuid = region.id;
        let svc = FakeServices::new()
            .with_region(region)
            .with_download("p.txt", "Cortex content")
            // Iter 1: off-target query — must be rejected with no search call.
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "bad".to_string(),
                name: "search_embeddings".to_string(),
                arguments: r#"{"query":"thalamus anatomy","top_k":3}"#.to_string(),
            }]))
            // Iter 2: on-target query — must succeed.
            .enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: "good".to_string(),
                name: "search_embeddings".to_string(),
                arguments: r#"{"query":"Cortex anatomy","top_k":3}"#.to_string(),
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
        let calls = svc.calls.lock().unwrap();
        // Exactly one search executed — the on-target one.
        assert_eq!(
            calls.searches.len(),
            1,
            "off-target query must NOT invoke vector search"
        );
    }

    #[tokio::test]
    async fn rag_loop_caps_consecutive_query_rejections_and_accepts_query() {
        // Model spams off-target queries. After the cap the guard must
        // stop rejecting so the loop can make progress instead of looping
        // forever or aborting (the rejection cap is there to bound waste,
        // not to abort the run).
        let region = sample_region(2);
        let region_uuid = region.id;
        let mut svc = FakeServices::new()
            .with_region(region)
            .with_download("p.txt", "Cortex content");
        // Enqueue MAX + 2 off-target tool calls + a final response. After
        // hitting the cap (currently 3) the guard accepts subsequent
        // off-target queries instead of rejecting them.
        for i in 0..6 {
            svc = svc.enqueue_summarize_ok(LlmResponse::ToolCalls(vec![ToolCall {
                id: format!("bad-{i}"),
                name: "search_embeddings".to_string(),
                arguments: r#"{"query":"thalamus anatomy","top_k":2}"#.to_string(),
            }]));
        }
        svc = svc.enqueue_summarize_ok(LlmResponse::Final("ok".to_string()));
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
        let calls = svc.calls.lock().unwrap();
        // First MAX_CONSECUTIVE_QUERY_REJECTIONS (= 3) are rejected, the
        // remaining 3 fall through to actual vector searches.
        assert_eq!(
            calls.searches.len(),
            3,
            "rejection cap must release subsequent off-target queries through to search"
        );
    }

    // ---------- Tests: Phase 4 — region template classification ----------

    #[test]
    fn region_template_classifies_cortical_layer_leaf() {
        let region = RegionMapping::new(1, "Primary somatosensory area, layer 5".to_string());
        assert_eq!(RegionTemplate::classify(&region), RegionTemplate::CorticalLayerLeaf);
        assert_eq!(RegionTemplate::classify(&region).label(), "cortical_layer_leaf");
    }

    #[test]
    fn region_template_classifies_tract_or_pathway() {
        for name in [
            "Corticospinal tract",
            "Anterior commissure",
            "Internal capsule",
            "Superior longitudinal fasciculus",
            "Cerebellar peduncle",
            "Calcarine fissure",
        ] {
            let region = RegionMapping::new(1, name.to_string());
            assert_eq!(
                RegionTemplate::classify(&region),
                RegionTemplate::TractOrPathway,
                "{name} must classify as TractOrPathway"
            );
        }
    }

    #[test]
    fn region_template_classifies_default_for_nuclei_and_areas() {
        for name in [
            "Hippocampus",
            "Lateral hypothalamic area",
            "Substantia nigra pars compacta",
            "Primary somatosensory area",
        ] {
            let region = RegionMapping::new(1, name.to_string());
            assert_eq!(
                RegionTemplate::classify(&region),
                RegionTemplate::Default,
                "{name} must classify as Default"
            );
        }
    }

    #[test]
    fn region_template_layer_leaf_template_excludes_disorders_section() {
        // Confirm the layer-leaf prompt does not encourage "Disorders" or
        // "Symptoms of Damage" sections that would invariably hallucinate.
        let prompt = RegionTemplate::CorticalLayerLeaf.system_template();
        assert!(
            !prompt.contains("## Associated Disorders"),
            "layer-leaf prompt must not include disorders section"
        );
        assert!(
            !prompt.contains("## Symptoms of Damage"),
            "layer-leaf prompt must not include symptoms section"
        );
    }

    #[test]
    fn region_template_tract_pathway_template_excludes_function_section() {
        let prompt = RegionTemplate::TractOrPathway.system_template();
        assert!(
            !prompt.contains("## Functions"),
            "tract/pathway prompt must not include functions section"
        );
        assert!(
            !prompt.contains("## Associated Disorders"),
            "tract/pathway prompt must not include disorders section"
        );
    }
}
