use crate::cache_keys::{self, cached_or_fetch, invalidate, invalidate_pattern};
use crate::{
    BatchManagement, CacheClient, EnvInfra, GenerateQueriesRequest, GenerateQueriesResponse,
    HttpClient, ServiceError,
};
use app::RegionManagement;
use domain::{
    BatchStatus, ChunkSourceResponse, ConfigKey, ProcessingBatch, RegionQuery, RegionSummary,
    SummaryEvalScores, SummarySource,
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

// ── Wire shapes for GET /evals-be/api/evals/scores/:summary_id ──
// Mirrored locally (instead of depending on evals-rpc-types) so orch stays
// decoupled from the evals-be crate graph.

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EvalsScoresWire {
    #[allow(dead_code)]
    pub summary_id: Uuid,
    #[serde(default)]
    pub scores: Vec<EvalsScoreEntryWire>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EvalsScoreEntryWire {
    pub metric: String,
    pub score: f32,
    pub eval_version: String,
    #[serde(default)]
    pub judge_model: Option<String>,
    /// ISO-8601 timestamp from evals-be. Used to pick the latest eval run when
    /// a summary has been scored under multiple eval_versions.
    #[serde(default)]
    pub created_at: String,
}

/// Concurrency cap for per-summary eval score fetches. Each summary costs one
/// cheap GET against evals-be (pure DB lookup, no LLM), so we keep this
/// reasonably high.
const EVAL_SCORES_FETCH_CONCURRENCY: usize = 16;

pub struct OrchRegionManagement<I> {
    infra: Arc<I>,
}

impl<I> OrchRegionManagement<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> RegionManagement for OrchRegionManagement<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E>
        + HttpClient<Error = E>
        + BatchManagement<Error = E>
        + crate::OrchDatabase<Error = E>
        + crate::RegionMappingQueries<Error = E>
        + CacheClient<Error = E>
        + Send
        + Sync,
{
    type Error = ServiceError<E>;

    async fn get_summaries(&self, region_id: Uuid) -> Result<Vec<RegionSummary>, Self::Error> {
        let infra = &self.infra;
        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::region_summaries(region_id),
            cache_keys::TTL_MEDIUM,
            || async {
                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                // First get the region mapping to find the Int4 region_id
                let region_mapping = infra
                    .get_region_mapping(&database_url, region_id)
                    .await
                    .map_err(ServiceError::InfraError)?
                    .ok_or_else(|| ServiceError::NotFound)?;

                // Query the region_summary table directly
                let summaries = infra
                    .get_region_summaries(&database_url, region_mapping.region_id)
                    .await
                    .map_err(ServiceError::InfraError)?;

                // For each summary, fetch its source chunks
                let mut result = Vec::with_capacity(summaries.len());
                for s in summaries {
                    let sources = infra
                        .get_summary_sources(&database_url, s.id)
                        .await
                        .map_err(ServiceError::InfraError)?;

                    result.push(RegionSummary {
                        summary_id: s.id,
                        summary: s.summary.unwrap_or_default(),
                        created_at: chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                            s.created_at,
                            chrono::Utc,
                        ),
                        batch_id: s.batch_id,
                        sources: sources
                            .into_iter()
                            .map(|src| SummarySource {
                                chunk_id: src.id,
                                pmc_id: src.source_pmc_id,
                                uid: src.source_uid,
                                source_query: src.source_query,
                            })
                            .collect(),
                        eval_scores: None,
                        cost_usd: None,
                    });
                }

                // Enrich each summary with its eval scores from evals-be. Fetches
                // run concurrently; a single failure just leaves that row's
                // `eval_scores` as `None` — it never fails the whole request.
                if !result.is_empty()
                    && let Ok(evals_base) =
                        resolve_evals_base_url(infra.as_ref(), &database_url).await
                {
                    let summary_ids: Vec<Uuid> = result.iter().map(|s| s.summary_id).collect();
                    let enriched: Vec<(Uuid, Option<SummaryEvalScores>)> =
                        stream::iter(summary_ids)
                            .map(|sid| {
                                let base = evals_base.clone();
                                let infra = Arc::clone(infra);
                                async move {
                                    (sid, fetch_summary_eval_scores(&*infra, &base, sid).await)
                                }
                            })
                            .buffer_unordered(EVAL_SCORES_FETCH_CONCURRENCY)
                            .collect()
                            .await;

                    let by_id: std::collections::HashMap<Uuid, Option<SummaryEvalScores>> =
                        enriched.into_iter().collect();
                    for s in result.iter_mut() {
                        s.eval_scores = by_id.get(&s.summary_id).and_then(|v| v.clone());
                    }
                }

                // Enrich each summary with its batch-level LLM cost from
                // brainatlas-be. Same best-effort semantics as eval scores:
                // a failure leaves `cost_usd = None`.
                if !result.is_empty()
                    && let Ok(brainatlas_base) =
                        resolve_brainatlas_base_url(infra.as_ref(), &database_url).await
                {
                    let batch_ids: Vec<Uuid> = result
                        .iter()
                        .map(|s| s.batch_id)
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();
                    let enriched_cost: Vec<(Uuid, Option<String>)> = stream::iter(batch_ids)
                        .map(|bid| {
                            let base = brainatlas_base.clone();
                            let infra = Arc::clone(infra);
                            async move { (bid, fetch_batch_cost_usd(&*infra, &base, bid).await) }
                        })
                        .buffer_unordered(EVAL_SCORES_FETCH_CONCURRENCY)
                        .collect()
                        .await;

                    let cost_by_batch: std::collections::HashMap<Uuid, Option<String>> =
                        enriched_cost.into_iter().collect();
                    for s in result.iter_mut() {
                        s.cost_usd = cost_by_batch.get(&s.batch_id).and_then(|v| v.clone());
                    }
                }

                Ok(result)
            },
        )
        .await
    }

    async fn get_active_batch(
        &self,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_active_batch(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_recent_batch(
        &self,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_recent_batch(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_queries(&self, region_id: Uuid) -> Result<Vec<RegionQuery>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_queries(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn update_batch_status(
        &self,
        batch_id: Uuid,
        status: BatchStatus,
        error_message: Option<String>,
    ) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .update_batch_status(&database_url, batch_id, status, error_message)
            .await
            .map_err(ServiceError::InfraError)?;

        // Invalidate batch-related caches
        invalidate(self.infra.as_ref(), &cache_keys::batch_status(batch_id)).await;
        invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;
        invalidate_pattern(self.infra.as_ref(), &cache_keys::batches_status_pattern()).await;

        Ok(())
    }

    async fn store_queries(
        &self,
        region_id: Uuid,
        queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .insert_queries(&database_url, region_id, queries)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, Self::Error> {
        tracing::info!(
            region_name,
            count,
            "Generating queries using LLM via brainatlas"
        );

        // Normalize URL helper
        fn normalize_url(addr: &str) -> String {
            if addr.starts_with("http://") || addr.starts_with("https://") {
                addr.to_string()
            } else {
                let replaced = addr.replace("0.0.0.0", "localhost");
                format!("http://{}", replaced)
            }
        }

        // Get brainatlas URL from env or config
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let brainatlas_url = match self.infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
            Ok(addr) => normalize_url(&addr),
            Err(_) => self
                .infra
                .get_config(&database_url, ConfigKey::BrainatlasBaseUrl)
                .await
                .map_err(ServiceError::InfraError)?
                .ok_or_else(|| ServiceError::ConfigNotFound {
                    key: "brainatlas_base_url".to_string(),
                })?,
        };

        let url = format!(
            "{}/brainatlas-be/api/generate-queries",
            brainatlas_url.trim_end_matches('/')
        );

        let request = GenerateQueriesRequest {
            region_name: region_name.to_string(),
            count,
            correlation_id: None,
        };

        tracing::info!(url = %url, region_name, count, "Calling brainatlas generate-queries endpoint");

        let response: GenerateQueriesResponse = self
            .infra
            .post(&url, &request)
            .await
            .map_err(ServiceError::InfraError)?;

        tracing::info!(
            region_name,
            query_count = response.queries.len(),
            queries = ?response.queries,
            "Successfully generated LLM queries"
        );

        Ok(response.queries)
    }

    async fn get_batches_by_status(
        &self,
        status: BatchStatus,
    ) -> Result<Vec<ProcessingBatch>, Self::Error> {
        let infra = &self.infra;
        let key = cache_keys::batches_by_status(status.as_str());
        cached_or_fetch(infra.as_ref(), &key, cache_keys::TTL_SHORT, || async {
            let database_url = infra
                .get_env_var("DATABASE_URL")
                .map_err(ServiceError::InfraError)?;

            infra
                .get_batches_by_status(&database_url, status)
                .await
                .map_err(ServiceError::InfraError)
        })
        .await
    }

    async fn get_region_name(&self, region_id: Uuid) -> Result<String, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let region = self
            .infra
            .get_region_mapping(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)?
            .ok_or_else(|| ServiceError::NotFound)?;

        Ok(region.name)
    }

    async fn get_total_regions(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_total_region_count(&database_url)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn count_regions_without_batches(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .count_regions_without_batches(&database_url)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn count_actively_fetching_regions(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .count_actively_fetching_regions(&database_url)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_latest_active_summary_age(
        &self,
        region_id: Uuid,
    ) -> Result<Option<chrono::NaiveDateTime>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .get_latest_active_summary_age(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)
    }

    async fn get_summary_freshness(&self) -> Result<domain::SummaryFreshness, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let staleness_days: i64 = self
            .infra
            .get_config(&database_url, ConfigKey::SummaryStalenessDays)
            .await
            .map_err(ServiceError::InfraError)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let counts = self
            .infra
            .get_summary_freshness_counts(&database_url, staleness_days)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(domain::SummaryFreshness {
            fresh: counts.fresh,
            stale: counts.stale,
            no_summary: counts.no_summary,
            staleness_days: counts.staleness_days,
        })
    }

    async fn get_query_generation_limit(&self) -> Result<Option<u32>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let value = self
            .infra
            .get_config(&database_url, ConfigKey::QueryGenerationLimit)
            .await
            .map_err(ServiceError::InfraError)?;

        Ok(value.and_then(|v| v.parse::<u32>().ok()))
    }

    async fn get_all_regions(&self) -> Result<Vec<domain::Region>, Self::Error> {
        let infra = &self.infra;
        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::all_regions(),
            cache_keys::TTL_LONG,
            || async {
                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                let regions = infra
                    .get_all_regions(&database_url)
                    .await
                    .map_err(ServiceError::InfraError)?;

                Ok(regions
                    .into_iter()
                    .map(|r| domain::Region {
                        id: r.id,
                        region_id: r.region_id,
                        name: r.name,
                        acronym: r.acronym,
                        color: if let (Some(red), Some(green), Some(blue)) =
                            (r.red, r.green, r.blue)
                        {
                            Some(domain::RegionColor { red, green, blue })
                        } else {
                            None
                        },
                        structure_order: r.structure_order,
                        parent_region_id: r.parent_region_id,
                        parent_acronym: r.parent_acronym,
                    })
                    .collect())
            },
        )
        .await
    }

    async fn delete_queries(&self, region_id: Uuid) -> Result<(), Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        self.infra
            .delete_queries(&database_url, region_id)
            .await
            .map_err(ServiceError::InfraError)?;

        // Invalidate region-level caches (queries deleted implies region reset)
        invalidate(
            self.infra.as_ref(),
            &cache_keys::region_summaries(region_id),
        )
        .await;
        invalidate(self.infra.as_ref(), &cache_keys::region_status(region_id)).await;
        invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;

        Ok(())
    }

    async fn delete_all_queries(&self) -> Result<i64, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let deleted = self
            .infra
            .delete_all_queries(&database_url)
            .await
            .map_err(ServiceError::InfraError)?;

        // All region queries are gone; drop global caches that might reflect them.
        invalidate(self.infra.as_ref(), &cache_keys::pipeline_stats()).await;

        Ok(deleted)
    }

    async fn get_chunk_source(&self, chunk_id: Uuid) -> Result<ChunkSourceResponse, Self::Error> {
        let infra = &self.infra;
        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::chunk_source(chunk_id),
            cache_keys::TTL_LONG,
            || async {
                // Normalize URL helper
                fn normalize_url(addr: &str) -> String {
                    if addr.starts_with("http://") || addr.starts_with("https://") {
                        addr.to_string()
                    } else {
                        let replaced = addr.replace("0.0.0.0", "localhost");
                        format!("http://{}", replaced)
                    }
                }

                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                let brainatlas_url = match infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
                    Ok(addr) => normalize_url(&addr),
                    Err(_) => {
                        infra
                            .get_config(&database_url, ConfigKey::BrainatlasBaseUrl)
                            .await
                            .map_err(ServiceError::InfraError)?
                            .ok_or_else(|| ServiceError::ConfigNotFound {
                                key: "brainatlas_base_url".to_string(),
                            })?
                    }
                };

                let url = format!(
                    "{}/brainatlas-be/api/chunks/{}/source",
                    brainatlas_url.trim_end_matches('/'),
                    chunk_id
                );

                tracing::info!(url = %url, %chunk_id, "Forwarding chunk source request to brainatlas");

                infra
                    .get::<ChunkSourceResponse>(&url)
                    .await
                    .map_err(ServiceError::InfraError)
            },
        )
        .await
    }

    async fn reverse_search(&self, query: &str) -> Result<domain::SearchResponse, Self::Error> {
        let infra = &self.infra;
        let database_url = infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // Read search result limit from config, default to 5
        let limit: i64 = match infra
            .get_config(&database_url, domain::ConfigKey::SearchResultLimit)
            .await
            .map_err(ServiceError::InfraError)?
        {
            Some(val) => val.parse::<i64>().unwrap_or(5),
            None => 5,
        };

        let query_owned = query.to_string();
        let db_url = database_url.clone();

        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::search_results(&query_owned),
            cache_keys::TTL_SHORT,
            || async {
                let (hits, total_count) = infra
                    .search_regions(&db_url, &query_owned, limit)
                    .await
                    .map_err(ServiceError::InfraError)?;

                let results = hits
                    .into_iter()
                    .map(|hit| domain::SearchResultItem {
                        region_id: hit.region_uuid,
                        region_numeric_id: hit.region_id,
                        name: hit.name,
                        acronym: hit.acronym,
                        summary_snippet: hit.summary_snippet,
                        match_source: hit.match_source,
                        rank: hit.rank,
                    })
                    .collect();

                Ok(domain::SearchResponse {
                    query: query_owned.clone(),
                    results,
                    total_found: total_count as usize,
                })
            },
        )
        .await
    }
}

// ── Eval score fetch helpers ───────────────────────────────────────────────

/// Resolve the evals-be base URL from env (`EVALS_BASE_URL`) then config
/// (`ConfigKey::EvalsBaseUrl`). Returns `Err` if neither is set — callers
/// should treat that as "eval enrichment disabled" rather than a hard
/// failure.
async fn resolve_evals_base_url<I, E>(
    infra: &I,
    database_url: &str,
) -> Result<String, ServiceError<E>>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + crate::OrchDatabase<Error = E> + Send + Sync,
{
    fn normalize_url(addr: &str) -> String {
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            let replaced = addr.replace("0.0.0.0", "localhost");
            format!("http://{}", replaced)
        }
    }

    if let Ok(url) = infra.get_env_var("EVALS_BASE_URL") {
        return Ok(normalize_url(&url));
    }

    let from_config = infra
        .get_config(database_url, ConfigKey::EvalsBaseUrl)
        .await
        .map_err(ServiceError::InfraError)?;
    if let Some(url) = from_config {
        return Ok(normalize_url(&url));
    }

    Err(ServiceError::ConfigNotFound {
        key: "evals_base_url (EVALS_BASE_URL env or evals_base_url config row)".to_string(),
    })
}

/// Fetch the eval scores for one summary. Returns:
/// - `Some(SummaryEvalScores)` iff evals-be has at least one score row for
///   this summary,
/// - `None` if the summary has never been evaluated (the `scores` array is
///   empty) OR the fetch itself failed (network / 5xx / decode error).
///
/// A failure to fetch must never abort the surrounding request — we just drop
/// the eval_scores field on the returned `RegionSummary`.
async fn fetch_summary_eval_scores<I, E>(
    infra: &I,
    evals_base: &str,
    summary_id: Uuid,
) -> Option<SummaryEvalScores>
where
    E: Error + Send + Sync + 'static,
    I: HttpClient<Error = E> + Send + Sync,
{
    let url = format!(
        "{}/evals-be/api/evals/scores/{}",
        evals_base.trim_end_matches('/'),
        summary_id
    );

    let wire: EvalsScoresWire = match infra.get(&url).await {
        Ok(w) => w,
        Err(e) => {
            tracing::debug!(%summary_id, error = %e, "evals-be scores fetch failed; omitting eval_scores");
            return None;
        }
    };

    // Key contract: only attach `eval_scores` when there's real data.
    // An empty `scores` array means the summary has never been evaluated.
    if wire.scores.is_empty() {
        return None;
    }

    // A summary can hold rows for multiple eval_versions side by side (the
    // unique index is on `(summary_hash, metric, eval_version)`, so bumping
    // `eval_version` adds rows instead of replacing them). We surface only
    // the most recently-run version so the frontend never shows a mix or
    // the wrong stale version.
    //
    // Strategy: group by version, pick the group whose newest row has the
    // largest `created_at` timestamp. Break ties lexicographically on the
    // version string (favours higher semver-like labels).
    use std::collections::HashMap;
    let mut by_version: HashMap<String, (String, Vec<EvalsScoreEntryWire>)> = HashMap::new();
    for entry in wire.scores {
        let group = by_version
            .entry(entry.eval_version.clone())
            .or_insert_with(|| (String::new(), Vec::new()));
        if entry.created_at > group.0 {
            group.0 = entry.created_at.clone();
        }
        group.1.push(entry);
    }

    let Some((eval_version, (_, entries))) = by_version
        .into_iter()
        .max_by(|a, b| a.1.0.cmp(&b.1.0).then_with(|| a.0.cmp(&b.0)))
    else {
        return None;
    };

    let mut scores = HashMap::with_capacity(entries.len());
    let mut judge_models = HashMap::new();
    for entry in entries {
        if let Some(m) = entry.judge_model {
            judge_models.insert(entry.metric.clone(), m);
        }
        scores.insert(entry.metric, entry.score);
    }

    Some(SummaryEvalScores {
        eval_version,
        scores,
        judge_models,
    })
}

// ── Brainatlas cost fetch helpers ─────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct UsageAggregateWire {
    #[serde(default)]
    pub total_cost_usd: f64,
    #[allow(dead_code)]
    #[serde(default)]
    pub total_calls: i64,
}

/// Resolve the brainatlas-be base URL from env (`BRAINATLAS_HTTP_ADDR`) then
/// config (`ConfigKey::BrainatlasBaseUrl`). Returns `Err` if neither is set.
async fn resolve_brainatlas_base_url<I, E>(
    infra: &I,
    database_url: &str,
) -> Result<String, ServiceError<E>>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + crate::OrchDatabase<Error = E> + Send + Sync,
{
    fn normalize_url(addr: &str) -> String {
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            let replaced = addr.replace("0.0.0.0", "localhost");
            format!("http://{}", replaced)
        }
    }

    if let Ok(url) = infra.get_env_var("BRAINATLAS_HTTP_ADDR") {
        return Ok(normalize_url(&url));
    }

    let from_config = infra
        .get_config(database_url, ConfigKey::BrainatlasBaseUrl)
        .await
        .map_err(ServiceError::InfraError)?;
    if let Some(url) = from_config {
        return Ok(normalize_url(&url));
    }

    Err(ServiceError::ConfigNotFound {
        key: "brainatlas_base_url (BRAINATLAS_HTTP_ADDR env or brainatlas_base_url config row)"
            .to_string(),
    })
}

/// Fetch total LLM cost attributed to a batch. Returns `None` on any failure
/// (network, 5xx, decode error) so a missing cost never aborts the summaries
/// request.
async fn fetch_batch_cost_usd<I, E>(
    infra: &I,
    brainatlas_base: &str,
    batch_id: Uuid,
) -> Option<String>
where
    E: Error + Send + Sync + 'static,
    I: HttpClient<Error = E> + Send + Sync,
{
    let url = format!(
        "{}/brainatlas-be/api/llm/usage?correlation_id=batch:{}",
        brainatlas_base.trim_end_matches('/'),
        batch_id
    );

    match infra.get::<UsageAggregateWire>(&url).await {
        Ok(agg) => Some(format!("{:.6}", agg.total_cost_usd)),
        Err(e) => {
            tracing::debug!(%batch_id, error = %e, "brainatlas-be cost fetch failed; omitting cost_usd");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::{
        ChunkSourceRecord, NewProcessedFetchTask, OrchConfig, PaperMetadataRecord,
        ProcessedFetchTask, RegionInfo, RegionMapping, RegionSummaryRecord, SearchHitRecord,
        SummaryFreshnessCounts, SystemStatsRaw,
    };
    use crate::{OrchDatabase, RegionMappingQueries};
    use async_trait::async_trait;
    use serde::de::DeserializeOwned;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(String);

    // ========== Wire-type serde round-trip (Task 1.7 first bullet) ==========

    #[test]
    fn evals_scores_wire_roundtrip_with_missing_optional_fields() {
        // judge_model and created_at are missing: must deserialize via
        // #[serde(default)].
        let sid = Uuid::new_v4();
        let json = serde_json::json!({
            "summary_id": sid,
            "scores": [
                {
                    "metric": "groundedness",
                    "score": 0.75,
                    "eval_version": "v0.2.0",
                },
                {
                    "metric": "rubric_relevance",
                    "score": 0.9,
                    "eval_version": "v0.2.0",
                    "judge_model": "gpt-4o-mini",
                    "created_at": "2026-04-20T12:00:00Z",
                }
            ]
        });

        let w: EvalsScoresWire = serde_json::from_value(json).expect("decode ok");
        assert_eq!(w.scores.len(), 2);
        assert_eq!(w.scores[0].metric, "groundedness");
        assert!(w.scores[0].judge_model.is_none());
        assert_eq!(w.scores[0].created_at, "");
        assert_eq!(w.scores[1].judge_model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(w.scores[1].created_at, "2026-04-20T12:00:00Z");

        // Round-trip: serializing then re-parsing must preserve content.
        let val = serde_json::to_value(&w).expect("ser");
        let w2: EvalsScoresWire = serde_json::from_value(val).expect("decode ok");
        assert_eq!(w2.scores.len(), 2);
    }

    #[test]
    fn evals_score_entry_numeric_edge_cases() {
        // 0.0 and 1.0 at boundaries
        let j = serde_json::json!({
            "summary_id": Uuid::nil(),
            "scores": [
                {"metric": "a", "score": 0.0, "eval_version": "v"},
                {"metric": "b", "score": 1.0, "eval_version": "v"},
                {"metric": "c", "score": -0.0, "eval_version": "v"},
            ]
        });
        let w: EvalsScoresWire = serde_json::from_value(j).unwrap();
        assert_eq!(w.scores[0].score, 0.0);
        assert_eq!(w.scores[1].score, 1.0);
        // negative zero also parses
        assert_eq!(w.scores[2].score, 0.0);
    }

    #[test]
    fn evals_score_entry_empty_scores_array_is_valid() {
        let j = serde_json::json!({
            "summary_id": Uuid::nil(),
            // missing "scores" — should default to []
        });
        let w: EvalsScoresWire = serde_json::from_value(j).unwrap();
        assert!(w.scores.is_empty());
    }

    // ========== Helper: HTTP fake that tracks concurrency ==========

    struct HelperInfra {
        responses: Mutex<HashMap<String, serde_json::Value>>,
        error_urls: Mutex<Vec<String>>, // URLs that should error
        delay_ms: AtomicUsize,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        call_count: AtomicUsize,
    }

    impl HelperInfra {
        fn new() -> Self {
            Self {
                responses: Mutex::new(HashMap::new()),
                error_urls: Mutex::new(vec![]),
                delay_ms: AtomicUsize::new(0),
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl HttpClient for HelperInfra {
        type Error = MockErr;

        async fn get<T: DeserializeOwned + Send>(
            &self,
            url: &str,
        ) -> Result<T, Self::Error> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.in_flight.fetch_add(1, Ordering::SeqCst);
            let cur = self.in_flight.load(Ordering::SeqCst);
            self.max_in_flight.fetch_max(cur, Ordering::SeqCst);

            let delay = self.delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay as u64))
                    .await;
            }

            let err = self.error_urls.lock().unwrap().iter().any(|p| url.contains(p));
            let result = if err {
                Err(MockErr(format!("staged error for {}", url)))
            } else {
                // Match by longest contained pattern.
                let map = self.responses.lock().unwrap();
                let mut best: Option<serde_json::Value> = None;
                let mut best_len = 0usize;
                for (pat, val) in map.iter() {
                    if url.contains(pat.as_str()) && pat.len() >= best_len {
                        best_len = pat.len();
                        best = Some(val.clone());
                    }
                }
                match best {
                    Some(v) => serde_json::from_value(v)
                        .map_err(|e| MockErr(format!("decode: {}", e))),
                    None => Err(MockErr(format!("no responder: {}", url))),
                }
            };

            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            result
        }

        async fn post<Req: serde::Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
            &self,
            _url: &str,
            _body: &Req,
        ) -> Result<Res, Self::Error> {
            unimplemented!()
        }

        async fn check_health(&self, _: &str, _: &str) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    // ========== fetch_summary_eval_scores unit tests ==========

    #[tokio::test]
    async fn fetch_eval_scores_returns_none_on_http_error() {
        let infra = HelperInfra::new();
        infra
            .error_urls
            .lock()
            .unwrap()
            .push("/scores/".to_string());
        let sid = Uuid::new_v4();
        let result =
            fetch_summary_eval_scores(&infra, "http://evals:8083", sid).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn fetch_eval_scores_returns_none_on_empty_scores_array() {
        let infra = HelperInfra::new();
        let sid = Uuid::new_v4();
        infra.responses.lock().unwrap().insert(
            format!("/scores/{}", sid),
            serde_json::json!({"summary_id": sid, "scores": []}),
        );
        let result =
            fetch_summary_eval_scores(&infra, "http://evals:8083", sid).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn fetch_eval_scores_picks_latest_eval_version_by_created_at() {
        let infra = HelperInfra::new();
        let sid = Uuid::new_v4();
        infra.responses.lock().unwrap().insert(
            format!("/scores/{}", sid),
            serde_json::json!({
                "summary_id": sid,
                "scores": [
                    {"metric": "groundedness", "score": 0.1, "eval_version": "v1",
                     "judge_model": "old-judge", "created_at": "2026-01-01T00:00:00Z"},
                    {"metric": "groundedness", "score": 0.9, "eval_version": "v2",
                     "judge_model": "new-judge", "created_at": "2026-04-01T00:00:00Z"},
                    {"metric": "rubric", "score": 0.5, "eval_version": "v2",
                     "created_at": "2026-04-01T00:00:00Z"},
                ]
            }),
        );
        let result =
            fetch_summary_eval_scores(&infra, "http://evals:8083", sid).await;
        let r = result.expect("some");
        assert_eq!(r.eval_version, "v2");
        assert_eq!(r.scores.len(), 2);
        assert_eq!(r.scores.get("groundedness").copied(), Some(0.9));
        assert_eq!(r.scores.get("rubric").copied(), Some(0.5));
        assert_eq!(
            r.judge_models.get("groundedness").map(|s| s.as_str()),
            Some("new-judge")
        );
        // "rubric" had no judge_model → should NOT be in judge_models.
        assert!(!r.judge_models.contains_key("rubric"));
    }

    #[tokio::test]
    async fn fetch_batch_cost_formats_total_cost_usd() {
        let infra = HelperInfra::new();
        let bid = Uuid::new_v4();
        infra.responses.lock().unwrap().insert(
            format!("correlation_id=batch:{}", bid),
            serde_json::json!({"total_cost_usd": 0.123456789_f64, "total_calls": 3}),
        );
        let c = fetch_batch_cost_usd(&infra, "http://brain:8082", bid).await;
        assert_eq!(c.as_deref(), Some("0.123457"));
    }

    #[tokio::test]
    async fn fetch_batch_cost_returns_none_on_error() {
        let infra = HelperInfra::new();
        infra
            .error_urls
            .lock()
            .unwrap()
            .push("/llm/usage".to_string());
        let c = fetch_batch_cost_usd(&infra, "http://brain:8082", Uuid::new_v4()).await;
        assert!(c.is_none());
    }

    // ========== Concurrency cap (EVAL_SCORES_FETCH_CONCURRENCY = 16) ==========
    //
    // The private constant is used by `stream::iter(...).buffer_unordered(16)`
    // only inside `get_summaries()`. We re-exercise the SAME pattern here with
    // the same constant over the helper, so any future bump to the cap will be
    // caught here too (asserts against `EVAL_SCORES_FETCH_CONCURRENCY`).
    #[tokio::test]
    async fn eval_scores_stream_respects_private_concurrency_constant() {
        // Ensure the constant is a reasonable positive number.
        assert!(EVAL_SCORES_FETCH_CONCURRENCY >= 1);

        let infra = HelperInfra::new();
        // 40 summary IDs; each call sleeps 20ms to create observable overlap.
        infra.delay_ms.store(20, Ordering::SeqCst);
        let ids: Vec<Uuid> = (0..40).map(|_| Uuid::new_v4()).collect();
        // One route that matches any /scores/<id>: use the evals API prefix.
        infra.responses.lock().unwrap().insert(
            "/evals-be/api/evals/scores/".to_string(),
            serde_json::json!({"summary_id": Uuid::nil(), "scores": []}),
        );

        let _results: Vec<_> = stream::iter(ids)
            .map(|sid| {
                let infra = &infra;
                async move {
                    fetch_summary_eval_scores(infra, "http://evals:8083", sid).await
                }
            })
            .buffer_unordered(EVAL_SCORES_FETCH_CONCURRENCY)
            .collect()
            .await;

        let peak = infra.max_in_flight.load(Ordering::SeqCst);
        assert!(
            peak <= EVAL_SCORES_FETCH_CONCURRENCY,
            "peak {} exceeded cap {}",
            peak,
            EVAL_SCORES_FETCH_CONCURRENCY,
        );
        // We staged 40 ids with 20ms each — parallelism must be substantial.
        assert!(
            peak >= 2,
            "expected observable parallelism, peak was only {}",
            peak
        );
        // All calls hit the mock.
        assert_eq!(infra.call_count.load(Ordering::SeqCst), 40);
    }

    // ========== get_summaries() cache hit ==========
    //
    // Full infra fake; only the cache_get responder matters. When the cache
    // returns a hit, no DB traits need to fire (the `cached_or_fetch` helper
    // short-circuits before the fetch_fn closure runs), so all the other
    // trait methods can safely be `unimplemented!()`.

    struct FullInfra {
        env: HashMap<String, String>,
        cache: Mutex<HashMap<String, String>>,
        cache_gets: Mutex<Vec<String>>,
        // For cache-miss path: DB records we return from
        // `get_region_mapping`, `get_region_summaries`, `get_summary_sources`.
        region_mapping: Mutex<Option<RegionMapping>>,
        summaries: Mutex<Vec<RegionSummaryRecord>>,
        summary_sources: Mutex<HashMap<Uuid, Vec<ChunkSourceRecord>>>,
    }

    impl FullInfra {
        fn new() -> Self {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_string(), "postgres://mock".to_string());
            Self {
                env,
                cache: Mutex::new(HashMap::new()),
                cache_gets: Mutex::new(vec![]),
                region_mapping: Mutex::new(None),
                summaries: Mutex::new(vec![]),
                summary_sources: Mutex::new(HashMap::new()),
            }
        }
    }

    impl EnvInfra for FullInfra {
        type Error = MockErr;
        fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
            self.env
                .get(key)
                .cloned()
                .ok_or_else(|| MockErr(format!("no env {}", key)))
        }
    }

    #[async_trait]
    impl OrchDatabase for FullInfra {
        type Error = MockErr;

        async fn get_config(
            &self,
            _: &str,
            _key: ConfigKey,
        ) -> Result<Option<String>, Self::Error> {
            // Return None so evals/brainatlas enrichment is skipped cleanly.
            Ok(None)
        }
        async fn get_processed_task(
            &self,
            _: &str,
            _: i64,
        ) -> Result<Option<ProcessedFetchTask>, Self::Error> {
            unimplemented!()
        }
        async fn insert_processed_task(
            &self,
            _: &str,
            _: NewProcessedFetchTask,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn update_brainatlas_status(
            &self,
            _: &str,
            _: i64,
            _: &str,
            _: Option<String>,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn get_all_config(
            &self,
            _: &str,
        ) -> Result<Vec<OrchConfig>, Self::Error> {
            unimplemented!()
        }
        async fn update_config(
            &self,
            _: &str,
            _: ConfigKey,
            _: &str,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl HttpClient for FullInfra {
        type Error = MockErr;
        async fn get<T: DeserializeOwned + Send>(&self, _url: &str) -> Result<T, Self::Error> {
            // No env/config for evals/brainatlas base URLs → these helpers
            // short-circuit before calling get(). We still return Err if hit,
            // to flag any unexpected enrichment paths.
            Err(MockErr("unexpected http GET".into()))
        }
        async fn post<Req: serde::Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
            &self,
            _url: &str,
            _body: &Req,
        ) -> Result<Res, Self::Error> {
            unimplemented!()
        }
        async fn check_health(&self, _: &str, _: &str) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl BatchManagement for FullInfra {
        type Error = MockErr;
        async fn get_queries(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Vec<domain::RegionQuery>, Self::Error> {
            unimplemented!()
        }
        async fn insert_queries(
            &self,
            _: &str,
            _: Uuid,
            _: Vec<String>,
        ) -> Result<Vec<Uuid>, Self::Error> {
            unimplemented!()
        }
        async fn delete_queries(&self, _: &str, _: Uuid) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn delete_all_queries(&self, _: &str) -> Result<i64, Self::Error> {
            unimplemented!()
        }
        async fn create_batch(&self, _: &str, _: Uuid, _: i32) -> Result<Uuid, Self::Error> {
            unimplemented!()
        }
        async fn add_tasks_to_batch(
            &self,
            _: &str,
            _: Uuid,
            _: Vec<i64>,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn update_batch_expected_count(
            &self,
            _: &str,
            _: Uuid,
            _: i32,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn get_batch_by_id(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<domain::ProcessingBatch>, Self::Error> {
            unimplemented!()
        }
        async fn get_batches_by_status(
            &self,
            _: &str,
            _: BatchStatus,
        ) -> Result<Vec<ProcessingBatch>, Self::Error> {
            unimplemented!()
        }
        async fn count_completed_tasks(
            &self,
            _: &str,
            _: &[i64],
        ) -> Result<usize, Self::Error> {
            unimplemented!()
        }
        async fn get_completed_task_ids(
            &self,
            _: &str,
            _: &[i64],
        ) -> Result<Vec<i64>, Self::Error> {
            unimplemented!()
        }
        async fn get_task_s3_keys(
            &self,
            _: &str,
            _: &[i64],
        ) -> Result<Vec<String>, Self::Error> {
            unimplemented!()
        }
        async fn get_task_paper_metadata(
            &self,
            _: &str,
            _: &[i64],
        ) -> Result<Vec<PaperMetadataRecord>, Self::Error> {
            unimplemented!()
        }
        async fn update_batch_status(
            &self,
            _: &str,
            _: Uuid,
            _: BatchStatus,
            _: Option<String>,
        ) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn complete_batch(&self, _: &str, _: Uuid) -> Result<(), Self::Error> {
            unimplemented!()
        }
        async fn get_active_batch(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<ProcessingBatch>, Self::Error> {
            unimplemented!()
        }
        async fn get_recent_batch(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<ProcessingBatch>, Self::Error> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl RegionMappingQueries for FullInfra {
        type Error = MockErr;
        async fn get_region_mapping(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<RegionMapping>, Self::Error> {
            Ok(self.region_mapping.lock().unwrap().clone())
        }
        async fn get_all_regions(
            &self,
            _: &str,
        ) -> Result<Vec<RegionMapping>, Self::Error> {
            unimplemented!()
        }
        async fn get_total_region_count(&self, _: &str) -> Result<i64, Self::Error> {
            unimplemented!()
        }
        async fn count_regions_without_batches(&self, _: &str) -> Result<i64, Self::Error> {
            unimplemented!()
        }
        async fn count_actively_fetching_regions(
            &self,
            _: &str,
        ) -> Result<i64, Self::Error> {
            unimplemented!()
        }
        async fn get_region_summaries(
            &self,
            _: &str,
            _region_id: i32,
        ) -> Result<Vec<RegionSummaryRecord>, Self::Error> {
            Ok(self.summaries.lock().unwrap().clone())
        }
        async fn get_summary_sources(
            &self,
            _: &str,
            summary_id: Uuid,
        ) -> Result<Vec<ChunkSourceRecord>, Self::Error> {
            Ok(self
                .summary_sources
                .lock()
                .unwrap()
                .get(&summary_id)
                .cloned()
                .unwrap_or_default())
        }
        async fn search_regions(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<(Vec<SearchHitRecord>, i64), Self::Error> {
            unimplemented!()
        }
        async fn get_regions_without_queries(
            &self,
            _: &str,
        ) -> Result<Vec<RegionInfo>, Self::Error> {
            unimplemented!()
        }
        async fn get_all_regions_with_queries(
            &self,
            _: &str,
        ) -> Result<Vec<(Uuid, String, Vec<String>)>, Self::Error> {
            unimplemented!()
        }
        async fn get_pending_fetch_task_count(&self, _: &str) -> Result<i64, Self::Error> {
            unimplemented!()
        }
        async fn get_latest_active_summary_age(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<chrono::NaiveDateTime>, Self::Error> {
            unimplemented!()
        }
        async fn get_summary_freshness_counts(
            &self,
            _: &str,
            _: i64,
        ) -> Result<SummaryFreshnessCounts, Self::Error> {
            unimplemented!()
        }
        async fn get_system_stats(
            &self,
            _: &str,
        ) -> Result<SystemStatsRaw, Self::Error> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl CacheClient for FullInfra {
        type Error = MockErr;
        async fn cache_get(&self, key: &str) -> Result<Option<String>, Self::Error> {
            self.cache_gets.lock().unwrap().push(key.to_string());
            Ok(self.cache.lock().unwrap().get(key).cloned())
        }
        async fn cache_set(
            &self,
            key: &str,
            val: &str,
            _ttl: u64,
        ) -> Result<(), Self::Error> {
            self.cache
                .lock()
                .unwrap()
                .insert(key.to_string(), val.to_string());
            Ok(())
        }
        async fn cache_del(&self, _key: &str) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn cache_del_pattern(&self, _pattern: &str) -> Result<u64, Self::Error> {
            Ok(0)
        }
        async fn cache_stats(&self) -> Result<domain::RedisStats, Self::Error> {
            Ok(domain::RedisStats {
                connected: true,
                error: None,
                total_keys: 0,
                keys_by_prefix: vec![],
                used_memory_bytes: 0,
                used_memory_human: "0B".to_string(),
                uptime_secs: 0,
                total_connections_received: 0,
                keyspace_hits: 0,
                keyspace_misses: 0,
                hit_rate: 0.0,
                server_version: "fake".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn get_summaries_cache_hit_short_circuits_db() {
        let region_id = Uuid::new_v4();
        let summary_id = Uuid::new_v4();
        let batch_id = Uuid::new_v4();

        // Pre-seed the cache with a serialized Vec<RegionSummary>.
        let cached = vec![domain::RegionSummary {
            summary_id,
            summary: "cached summary".to_string(),
            created_at: chrono::Utc::now(),
            batch_id,
            sources: vec![],
            eval_scores: None,
            cost_usd: None,
        }];
        let infra = FullInfra::new();
        infra.cache.lock().unwrap().insert(
            crate::cache_keys::region_summaries(region_id),
            serde_json::to_string(&cached).unwrap(),
        );
        // NOTE: region_mapping intentionally NOT set. A cache miss here would
        // return NotFound from `get_region_mapping` via Ok(None), proving
        // the hit path was taken.
        let infra = Arc::new(infra);
        let svc = OrchRegionManagement::new(infra.clone());

        let out = <OrchRegionManagement<FullInfra> as app::RegionManagement>::get_summaries(
            &svc, region_id,
        )
        .await
        .expect("cache hit");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary_id, summary_id);
        assert_eq!(out[0].summary, "cached summary");

        // Cache was consulted for the expected key.
        let gets = infra.cache_gets.lock().unwrap().clone();
        assert!(gets.contains(&crate::cache_keys::region_summaries(region_id)));
    }

    #[tokio::test]
    async fn get_summaries_cache_miss_not_found_when_region_mapping_missing() {
        // Cache is empty and region_mapping is None → NotFound error.
        let region_id = Uuid::new_v4();
        let infra = Arc::new(FullInfra::new());
        let svc = OrchRegionManagement::new(infra.clone());

        let err = <OrchRegionManagement<FullInfra> as app::RegionManagement>::get_summaries(
            &svc, region_id,
        )
        .await
        .err()
        .expect("err");
        match err {
            ServiceError::NotFound => {}
            other => panic!("expected NotFound, got {:?}", other),
        }
        // Confirm we did go through the cache miss path (cache_get was invoked).
        let gets = infra.cache_gets.lock().unwrap().clone();
        assert_eq!(gets.len(), 1);
        assert_eq!(gets[0], crate::cache_keys::region_summaries(region_id));
    }

    #[tokio::test]
    async fn get_summaries_cache_miss_populates_from_db_then_caches() {
        let region_id = Uuid::new_v4();
        let summary_id = Uuid::new_v4();
        let batch_id = Uuid::new_v4();

        let infra = FullInfra::new();
        *infra.region_mapping.lock().unwrap() = Some(RegionMapping {
            id: region_id,
            region_id: 42,
            name: "hippocampus".to_string(),
            acronym: None,
            red: None,
            green: None,
            blue: None,
            structure_order: None,
            parent_region_id: None,
            parent_acronym: None,
        });
        *infra.summaries.lock().unwrap() = vec![RegionSummaryRecord {
            id: summary_id,
            summary: Some("hello".to_string()),
            created_at: chrono::NaiveDateTime::parse_from_str(
                "2026-04-20 10:00:00",
                "%Y-%m-%d %H:%M:%S",
            )
            .unwrap(),
            batch_id,
        }];
        let infra = Arc::new(infra);
        let svc = OrchRegionManagement::new(infra.clone());

        let out = <OrchRegionManagement<FullInfra> as app::RegionManagement>::get_summaries(
            &svc, region_id,
        )
        .await
        .expect("ok");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary, "hello");
        assert!(out[0].eval_scores.is_none());
        assert!(out[0].cost_usd.is_none());

        // After fetch, the cache should hold a JSON string for this key.
        let cache = infra.cache.lock().unwrap();
        assert!(cache.contains_key(&crate::cache_keys::region_summaries(region_id)));
    }
}
