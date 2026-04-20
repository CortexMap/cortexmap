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
                    });
                }

                // Enrich each summary with its eval scores from evals-be. Fetches
                // run concurrently; a single failure just leaves that row's
                // `eval_scores` as `None` — it never fails the whole request.
                if !result.is_empty()
                    && let Ok(evals_base) =
                        resolve_evals_base_url(infra.as_ref(), &database_url).await
                {
                    let summary_ids: Vec<Uuid> =
                        result.iter().map(|s| s.summary_id).collect();
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

    // All scores for a given summary share the same eval_version (enforced by
    // the eval_scores unique index (summary_hash, metric, eval_version)).
    // Pick the first one's version as the representative.
    let eval_version = wire
        .scores
        .first()
        .map(|s| s.eval_version.clone())
        .unwrap_or_default();

    let mut scores = std::collections::HashMap::with_capacity(wire.scores.len());
    let mut judge_models = std::collections::HashMap::new();
    for entry in wire.scores {
        if let Some(m) = entry.judge_model.clone() {
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
