use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Priority levels for fetch/process tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Background,    // 0 - Routine scheduled scan
    Normal,        // 5 - Standard enqueue
    UserRequested, // 8 - Triggered by search miss
    Invalidation,  // 10 - Force refresh
}

impl Priority {
    pub fn as_i32(&self) -> i32 {
        match self {
            Priority::Background => 0,
            Priority::Normal => 5,
            Priority::UserRequested => 8,
            Priority::Invalidation => 10,
        }
    }
}

/// End-to-end pipeline state for a brain region
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionPipelineStatus {
    NotStarted,  // No fetch task exists
    FetchQueued, // Fetch task exists, status = pending
    Fetching,    // Fetch task status = in_progress
    FetchFailed, // Fetch task status = failed
    LlmQueued,   // Fetch complete, handed to brainatlas, not done yet
    Processing,  // Brainatlas is chunking/embedding/summarizing
    Done,        // At least one region_summary entry exists
    Invalidated, // New cycle queued on top of existing summaries
}

/// Source chunk referenced by a summary (lightweight metadata for client)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarySource {
    /// UUID of the brain_region_embeddings row (chunk identifier)
    pub chunk_id: Uuid,
    /// PMC ID of the source paper, if available
    pub pmc_id: Option<String>,
    /// UID of the source paper, if available
    pub uid: Option<String>,
    /// Query that led to fetching this source
    pub source_query: Option<String>,
}

/// A single region summary entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionSummary {
    /// `region_summary.id` — also used as the evals lookup key.
    pub summary_id: Uuid,
    pub summary: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// The batch that generated this summary
    pub batch_id: Uuid,
    /// Source chunks used to generate this summary
    pub sources: Vec<SummarySource>,
    /// Eval scores for this summary, if any have been computed.
    /// `None` means evals-be has never scored this summary (or the fetch
    /// failed). When present, the map is keyed by metric name
    /// (e.g. `"claim_groundedness"`, `"rubric_relevance"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_scores: Option<SummaryEvalScores>,
    /// Total LLM cost in USD attributed to this batch (sum of all LLM calls
    /// whose `correlation_id == "batch:{batch_id}"`). `None` when the cost
    /// could not be fetched from brainatlas-be (enrichment best-effort).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<String>,
}

/// Compact per-metric score payload attached to a `RegionSummary`.
///
/// This is a condensed view of evals-be's `GET /api/evals/scores/:summary_id`
/// response: we keep just the score values + judge models (dropping the
/// verbose per-claim `details` JSON), because the summaries route is hot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryEvalScores {
    pub eval_version: String,
    /// metric_name -> score in [0, 1]
    pub scores: std::collections::HashMap<String, f32>,
    /// metric_name -> model identifier (only set for LLM-judged metrics)
    #[serde(default)]
    pub judge_models: std::collections::HashMap<String, String>,
}

/// Result of searching/listing summaries for a region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRegionResult {
    /// All summaries for this region, ordered by creation time (newest first)
    pub summaries: Vec<RegionSummary>,
}

/// Result of creating a new summary generation batch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateSummaryResult {
    /// The newly created batch ID (or existing active batch ID if already in progress)
    pub batch_id: Uuid,
    /// Number of queries generated
    pub query_count: usize,
    /// Number of fetch tasks enqueued
    pub task_count: usize,
    /// Whether a new batch was created or an existing one was returned
    pub already_in_progress: bool,
}

/// Current status of a batch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStatusResult {
    /// Batch ID
    pub batch_id: Uuid,
    /// Current batch status
    pub status: RegionPipelineStatus,
    /// Progress message (e.g., "Fetching papers: 10/20 complete")
    pub message: String,
    /// Error message if failed
    pub error: Option<String>,
    /// Expected task count
    pub expected_tasks: i32,
    /// Completed task count (if available)
    pub completed_tasks: Option<i32>,
    /// Created timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Result of getting region status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionStatusResult {
    pub region_id: Uuid,
    pub status: RegionPipelineStatus,
    pub last_fetch_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_summary_at: Option<chrono::DateTime<chrono::Utc>>,
    pub summary_count: i32,
    pub current_priority: Option<Priority>,
}

/// Result of invalidating a region (deprecated, use /generate instead)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvalidateResult {
    pub region_id: Uuid,
    pub new_status: RegionPipelineStatus,
    pub detail: String,
}

/// Pipeline statistics across all regions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStatsResult {
    pub total_regions: i32,
    pub not_started: i32,
    pub fetch_queued: i32,
    pub fetching: i32,
    pub fetch_failed: i32,
    pub llm_queued: i32,
    pub processing: i32,
    pub done: i32,
    pub invalidated: i32,
}

/// A configuration entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub description: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Update for a configuration entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntryUpdate {
    pub key: String,
    pub value: String,
}

/// Brain region from region_mapping table
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub id: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub color: Option<RegionColor>,
    pub structure_order: Option<i32>,
    pub parent_region_id: Option<i32>,
    pub parent_acronym: Option<String>,
}

/// Full source details for a chunk (returned by chunk source resolution endpoint)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSourceResponse {
    pub chunk_id: Uuid,
    pub chunk_text: String,
    pub source_s3_key: Option<String>,
    pub source_pmc_id: Option<String>,
    pub source_uid: Option<String>,
    pub source_query: Option<String>,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
}

/// RGB color representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionColor {
    pub red: i32,
    pub green: i32,
    pub blue: i32,
}

/// Request body for the reverse search endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    /// The natural language search input
    pub query: String,
}

/// A single item in the reverse search results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultItem {
    /// The region_mapping UUID
    pub region_id: Uuid,
    /// The integer region_id
    pub region_numeric_id: i32,
    /// Region name
    pub name: String,
    /// Region shortform / acronym
    pub acronym: Option<String>,
    /// Truncated summary text (~200 chars) if matched via summary
    pub summary_snippet: Option<String>,
    /// What matched: "name", "acronym", or "summary"
    pub match_source: String,
    /// Relevance score (higher = better match)
    pub rank: f64,
}

/// Response from the reverse search endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Echo back the input query
    pub query: String,
    /// The matching results
    pub results: Vec<SearchResultItem>,
    /// How many total matches existed before the LIMIT was applied
    pub total_found: usize,
}

/// Lightweight real-time pipeline health snapshot (no shared state required)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineHealthStatus {
    /// Regions that still need query generation (Phase 1 backlog)
    pub regions_without_queries: usize,
    /// Regions that have queries and are eligible for paper discovery
    pub regions_with_queries: usize,
    /// Fetch tasks currently pending or in-progress
    pub pending_fetch_tasks: i64,
    /// Active (running) fetcher workers
    pub worker_count: usize,
}

/// Per-region summary freshness buckets, surfaced on `/dev/api/summary-freshness`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SummaryFreshness {
    /// Regions whose latest active summary is younger than `staleness_days`.
    pub fresh: i64,
    /// Regions whose latest active summary is older than `staleness_days`.
    pub stale: i64,
    /// Regions with no usable active summary.
    pub no_summary: i64,
    /// Cutoff used to bucket the rows.
    pub staleness_days: i64,
}

/// Comprehensive system stats for the /dev/stats dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    /// Fetch tasks grouped by status
    pub fetch_tasks_by_status: Vec<StatusCount>,
    /// Processing batches grouped by status
    pub batches_by_status: Vec<StatusCount>,
    /// Total queries in region_queries
    pub total_queries: i64,
    /// Number of distinct regions that have queries
    pub regions_with_queries: i64,
    /// Query count distribution (e.g. [{"count": 3, "num_regions": 769}])
    pub query_distribution: Vec<QueryDistEntry>,
    /// Total papers in papers table
    pub total_papers: i64,
    /// Total region summaries
    pub total_summaries: i64,
    /// Server uptime timestamp (when this response was generated)
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCount {
    pub status: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryDistEntry {
    pub query_count: i64,
    pub num_regions: i64,
}

/// Request body for manually triggering a pipeline cycle.
/// All fields default to `false` so `POST /orch/api/pipeline/trigger` with `{}`
/// is a safe no-op. Clients opt into each step explicitly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineTriggerRequest {
    /// Wipe every row from `region_queries` before generating. Only use when you
    /// really want a full reset -- this forces Phase 1 to regenerate everything.
    #[serde(default)]
    pub reset_queries: bool,
    /// Run Phase 1: generate queries for regions that don't have any.
    /// Combined with `reset_queries` this regenerates queries for ALL regions.
    #[serde(default)]
    pub generate_queries: bool,
    /// Run Phase 2: re-scan NCBI for every region's queries, enqueue new papers.
    #[serde(default)]
    pub discover_papers: bool,
    /// Run Phase 3: ensure fetcher workers are allocated so the queue drains.
    #[serde(default)]
    pub ensure_workers: bool,
}

/// Response with per-phase outcomes. Any phase that wasn't requested stays `None`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineTriggerResult {
    /// Number of rows deleted by the reset (0 if `reset_queries` was false).
    pub reset_queries_deleted: Option<i64>,
    /// Phase 1 result: (regions_processed, total_queries_generated).
    pub generate_queries_result: Option<(usize, usize)>,
    /// Phase 2 result: (regions_scanned, new_tasks_created).
    pub discover_papers_result: Option<(usize, usize)>,
    /// Phase 3 result: true when the call succeeded.
    pub ensure_workers_ok: Option<bool>,
    /// Any non-fatal errors, one per failed phase.
    pub errors: Vec<String>,
}

/// Snapshot of the Redis cache used by orch. Note: orch uses Redis only as a
/// key-value cache (no list/queue ops). The actual task queue lives in
/// PostgreSQL `fetch_tasks`. This endpoint surfaces cache health and
/// per-prefix key counts for observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisStats {
    /// `true` if PING succeeded, `false` otherwise.
    pub connected: bool,
    /// Last error message if the connection failed (None on success).
    pub error: Option<String>,
    /// Total keys across the entire Redis DB (from DBSIZE).
    pub total_keys: u64,
    /// Per-prefix key counts (e.g. `{"orch:region:*:status": 1198}`).
    /// Only populated when connected.
    pub keys_by_prefix: Vec<RedisPrefixCount>,
    /// Memory usage in bytes (from INFO `used_memory`). 0 on failure.
    pub used_memory_bytes: u64,
    /// Human-readable memory string (e.g. "12.4M").
    pub used_memory_human: String,
    /// Server uptime in seconds (from INFO `uptime_in_seconds`). 0 on failure.
    pub uptime_secs: u64,
    /// Total connections received since boot.
    pub total_connections_received: u64,
    /// Cumulative cache hits / misses since boot.
    pub keyspace_hits: u64,
    pub keyspace_misses: u64,
    /// Hit rate as a 0..=1 fraction (computed from hits / (hits+misses)).
    pub hit_rate: f64,
    /// Redis server version (e.g. "7.2.4").
    pub server_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisPrefixCount {
    /// Glob pattern scanned (e.g. `orch:region:*:status`).
    pub pattern: String,
    /// Human-friendly description of what's stored under this prefix.
    pub description: String,
    /// Number of keys matching the pattern.
    pub count: u64,
}

/// Aggregate LLM cost for a single eval run. Returned by
/// `GET /orch/api/evals/runs/{run_id}/cost`. Orch queries brainatlas-be's
/// `/api/llm/usage?correlation_id_prefix=eval:{run_id}:` and forwards the
/// result. Scalars are strings (not floats) to preserve full precision
/// without a parse round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRunCost {
    pub run_id: String,
    pub total_cost_usd: String,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_calls: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::collections::HashMap;

    fn ts() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 4, 20, 12, 0, 0).unwrap()
    }

    // ---- Priority ----

    #[test]
    fn priority_roundtrips_all_variants() {
        for p in [
            Priority::Background,
            Priority::Normal,
            Priority::UserRequested,
            Priority::Invalidation,
        ] {
            let v = serde_json::to_value(p).unwrap();
            let back: Priority = serde_json::from_value(v).unwrap();
            assert_eq!(p, back);
        }
    }

    #[test]
    fn priority_serializes_by_variant_name() {
        assert_eq!(
            serde_json::to_value(Priority::Background).unwrap(),
            "Background"
        );
        assert_eq!(serde_json::to_value(Priority::Normal).unwrap(), "Normal");
        assert_eq!(
            serde_json::to_value(Priority::UserRequested).unwrap(),
            "UserRequested"
        );
        assert_eq!(
            serde_json::to_value(Priority::Invalidation).unwrap(),
            "Invalidation"
        );
    }

    #[test]
    fn priority_as_i32_values_match_comments() {
        assert_eq!(Priority::Background.as_i32(), 0);
        assert_eq!(Priority::Normal.as_i32(), 5);
        assert_eq!(Priority::UserRequested.as_i32(), 8);
        assert_eq!(Priority::Invalidation.as_i32(), 10);
    }

    // ---- RegionPipelineStatus ----

    #[test]
    fn region_pipeline_status_roundtrips_all_variants() {
        for s in [
            RegionPipelineStatus::NotStarted,
            RegionPipelineStatus::FetchQueued,
            RegionPipelineStatus::Fetching,
            RegionPipelineStatus::FetchFailed,
            RegionPipelineStatus::LlmQueued,
            RegionPipelineStatus::Processing,
            RegionPipelineStatus::Done,
            RegionPipelineStatus::Invalidated,
        ] {
            let v = serde_json::to_value(s).unwrap();
            let back: RegionPipelineStatus = serde_json::from_value(v).unwrap();
            assert_eq!(s, back);
        }
    }

    // ---- SummarySource ----

    #[test]
    fn summary_source_roundtrip_full() {
        let src = SummarySource {
            chunk_id: Uuid::new_v4(),
            pmc_id: Some("PMC123".to_string()),
            uid: Some("UID-1".to_string()),
            source_query: Some("hippocampus memory".to_string()),
        };
        let v = serde_json::to_value(&src).unwrap();
        let back: SummarySource = serde_json::from_value(v).unwrap();
        assert_eq!(back.chunk_id, src.chunk_id);
        assert_eq!(back.pmc_id, src.pmc_id);
        assert_eq!(back.uid, src.uid);
        assert_eq!(back.source_query, src.source_query);
    }

    #[test]
    fn summary_source_roundtrip_with_nulls() {
        let src = SummarySource {
            chunk_id: Uuid::new_v4(),
            pmc_id: None,
            uid: None,
            source_query: None,
        };
        let v = serde_json::to_value(&src).unwrap();
        // These are plain Option fields (no skip_serializing_if): they stay as nulls.
        assert!(v.get("pmc_id").is_some());
        let back: SummarySource = serde_json::from_value(v).unwrap();
        assert!(back.pmc_id.is_none());
        assert!(back.uid.is_none());
        assert!(back.source_query.is_none());
    }

    // ---- RegionSummary + SummaryEvalScores ----

    fn sample_region_summary(
        eval_scores: Option<SummaryEvalScores>,
        cost: Option<String>,
    ) -> RegionSummary {
        RegionSummary {
            summary_id: Uuid::new_v4(),
            summary: "A brief overview.".to_string(),
            created_at: ts(),
            batch_id: Uuid::new_v4(),
            sources: vec![SummarySource {
                chunk_id: Uuid::new_v4(),
                pmc_id: Some("PMC9".to_string()),
                uid: None,
                source_query: None,
            }],
            eval_scores,
            cost_usd: cost,
        }
    }

    #[test]
    fn region_summary_roundtrip_full() {
        let mut scores = HashMap::new();
        scores.insert("claim_groundedness".to_string(), 0.75_f32);
        let mut judges = HashMap::new();
        judges.insert("claim_groundedness".to_string(), "gpt-4o-mini".to_string());
        let rs = sample_region_summary(
            Some(SummaryEvalScores {
                eval_version: "v2".to_string(),
                scores,
                judge_models: judges,
            }),
            Some("0.1234".to_string()),
        );
        let v = serde_json::to_value(&rs).unwrap();
        assert_eq!(v["cost_usd"], "0.1234");
        assert_eq!(v["eval_scores"]["eval_version"], "v2");
        let back: RegionSummary = serde_json::from_value(v).unwrap();
        assert_eq!(back.summary_id, rs.summary_id);
        assert_eq!(back.cost_usd, rs.cost_usd);
        let back_scores = back.eval_scores.unwrap();
        assert_eq!(back_scores.eval_version, "v2");
        assert_eq!(back_scores.scores.get("claim_groundedness"), Some(&0.75));
        assert_eq!(
            back_scores.judge_models.get("claim_groundedness").unwrap(),
            "gpt-4o-mini"
        );
    }

    #[test]
    fn region_summary_skips_eval_scores_and_cost_when_none() {
        let rs = sample_region_summary(None, None);
        let v = serde_json::to_value(&rs).unwrap();
        assert!(v.get("eval_scores").is_none());
        assert!(v.get("cost_usd").is_none());
        // But required fields are present.
        assert!(v.get("summary_id").is_some());
        assert!(v.get("summary").is_some());
        assert!(v.get("batch_id").is_some());
        assert!(v.get("sources").is_some());
    }

    #[test]
    fn summary_eval_scores_defaults_judge_models_to_empty_map() {
        // Legacy payloads without judge_models must still deserialize.
        let json = r#"{"eval_version":"v1","scores":{"rubric_relevance":0.5}}"#;
        let parsed: SummaryEvalScores = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.eval_version, "v1");
        assert_eq!(parsed.scores.get("rubric_relevance"), Some(&0.5));
        assert!(parsed.judge_models.is_empty());
    }

    // ---- SearchRegionResult ----

    #[test]
    fn search_region_result_roundtrip() {
        let r = SearchRegionResult {
            summaries: vec![sample_region_summary(None, None)],
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: SearchRegionResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.summaries.len(), 1);
    }

    #[test]
    fn search_region_result_empty_roundtrip() {
        let r = SearchRegionResult { summaries: vec![] };
        let v = serde_json::to_value(&r).unwrap();
        let back: SearchRegionResult = serde_json::from_value(v).unwrap();
        assert!(back.summaries.is_empty());
    }

    // ---- GenerateSummaryResult ----

    #[test]
    fn generate_summary_result_roundtrip() {
        let r = GenerateSummaryResult {
            batch_id: Uuid::new_v4(),
            query_count: 12,
            task_count: 20,
            already_in_progress: false,
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: GenerateSummaryResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.batch_id, r.batch_id);
        assert_eq!(back.query_count, 12);
        assert_eq!(back.task_count, 20);
        assert!(!back.already_in_progress);
    }

    // ---- BatchStatusResult ----

    #[test]
    fn batch_status_result_roundtrip() {
        let r = BatchStatusResult {
            batch_id: Uuid::new_v4(),
            status: RegionPipelineStatus::Processing,
            message: "Fetching papers: 10/20 complete".to_string(),
            error: None,
            expected_tasks: 20,
            completed_tasks: Some(10),
            created_at: ts(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["status"], "Processing");
        let back: BatchStatusResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.batch_id, r.batch_id);
        assert_eq!(back.status, RegionPipelineStatus::Processing);
        assert_eq!(back.completed_tasks, Some(10));
    }

    // ---- RegionStatusResult ----

    #[test]
    fn region_status_result_roundtrip_with_priority() {
        let r = RegionStatusResult {
            region_id: Uuid::new_v4(),
            status: RegionPipelineStatus::Done,
            last_fetch_at: Some(ts()),
            last_summary_at: Some(ts()),
            summary_count: 3,
            current_priority: Some(Priority::UserRequested),
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: RegionStatusResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.region_id, r.region_id);
        assert_eq!(back.status, RegionPipelineStatus::Done);
        assert_eq!(back.current_priority, Some(Priority::UserRequested));
        assert_eq!(back.summary_count, 3);
    }

    // ---- InvalidateResult ----

    #[test]
    fn invalidate_result_roundtrip() {
        let r = InvalidateResult {
            region_id: Uuid::new_v4(),
            new_status: RegionPipelineStatus::Invalidated,
            detail: "queued".to_string(),
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: InvalidateResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.region_id, r.region_id);
        assert_eq!(back.new_status, RegionPipelineStatus::Invalidated);
        assert_eq!(back.detail, "queued");
    }

    // ---- PipelineStatsResult ----

    #[test]
    fn pipeline_stats_result_roundtrip() {
        let r = PipelineStatsResult {
            total_regions: 1200,
            not_started: 100,
            fetch_queued: 50,
            fetching: 10,
            fetch_failed: 5,
            llm_queued: 40,
            processing: 20,
            done: 975,
            invalidated: 0,
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: PipelineStatsResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.total_regions, 1200);
        assert_eq!(back.done, 975);
    }

    // ---- ConfigEntry / ConfigEntryUpdate ----

    #[test]
    fn config_entry_roundtrip() {
        let e = ConfigEntry {
            key: "chat_model".to_string(),
            value: "openai/gpt-4o-mini".to_string(),
            description: Some("Chat model".to_string()),
            updated_at: ts(),
        };
        let v = serde_json::to_value(&e).unwrap();
        let back: ConfigEntry = serde_json::from_value(v).unwrap();
        assert_eq!(back.key, e.key);
        assert_eq!(back.value, e.value);
        assert_eq!(back.description, e.description);
    }

    #[test]
    fn config_entry_update_roundtrip() {
        let u = ConfigEntryUpdate {
            key: "chat_model".to_string(),
            value: "anthropic/claude".to_string(),
        };
        let v = serde_json::to_value(&u).unwrap();
        let back: ConfigEntryUpdate = serde_json::from_value(v).unwrap();
        assert_eq!(back.key, u.key);
        assert_eq!(back.value, u.value);
    }

    // ---- Region / RegionColor ----

    #[test]
    fn region_color_roundtrip() {
        let c = RegionColor {
            red: 10,
            green: 200,
            blue: 30,
        };
        let v = serde_json::to_value(&c).unwrap();
        let back: RegionColor = serde_json::from_value(v).unwrap();
        assert_eq!(back.red, 10);
        assert_eq!(back.green, 200);
        assert_eq!(back.blue, 30);
    }

    #[test]
    fn region_roundtrip_full() {
        let r = Region {
            id: Uuid::new_v4(),
            region_id: 42,
            name: "Hippocampus".to_string(),
            acronym: Some("HPF".to_string()),
            color: Some(RegionColor {
                red: 1,
                green: 2,
                blue: 3,
            }),
            structure_order: Some(5),
            parent_region_id: Some(1),
            parent_acronym: Some("CTX".to_string()),
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: Region = serde_json::from_value(v).unwrap();
        assert_eq!(back.id, r.id);
        assert_eq!(back.name, "Hippocampus");
        assert_eq!(back.color.as_ref().unwrap().green, 2);
    }

    #[test]
    fn region_roundtrip_nullable_fields() {
        let r = Region {
            id: Uuid::new_v4(),
            region_id: 1,
            name: "Root".to_string(),
            acronym: None,
            color: None,
            structure_order: None,
            parent_region_id: None,
            parent_acronym: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: Region = serde_json::from_value(v).unwrap();
        assert!(back.acronym.is_none());
        assert!(back.color.is_none());
    }

    // ---- ChunkSourceResponse ----

    #[test]
    fn chunk_source_response_roundtrip() {
        let c = ChunkSourceResponse {
            chunk_id: Uuid::new_v4(),
            chunk_text: "Some text".to_string(),
            source_s3_key: Some("s3://key".to_string()),
            source_pmc_id: Some("PMC1".to_string()),
            source_uid: None,
            source_query: Some("q".to_string()),
            char_start: Some(0),
            char_end: Some(9),
        };
        let v = serde_json::to_value(&c).unwrap();
        let back: ChunkSourceResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.chunk_text, "Some text");
        assert_eq!(back.char_end, Some(9));
    }

    // ---- SearchRequest / SearchResultItem / SearchResponse ----

    #[test]
    fn search_request_roundtrip() {
        let r = SearchRequest {
            query: "memory".to_string(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["query"], "memory");
        let back: SearchRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.query, "memory");
    }

    #[test]
    fn search_result_item_roundtrip() {
        let item = SearchResultItem {
            region_id: Uuid::new_v4(),
            region_numeric_id: 7,
            name: "Hippocampus".to_string(),
            acronym: Some("HPF".to_string()),
            summary_snippet: Some("Supports memory...".to_string()),
            match_source: "summary".to_string(),
            rank: 0.87,
        };
        let v = serde_json::to_value(&item).unwrap();
        let back: SearchResultItem = serde_json::from_value(v).unwrap();
        assert_eq!(back.region_numeric_id, 7);
        assert_eq!(back.match_source, "summary");
        assert!((back.rank - 0.87).abs() < 1e-9);
    }

    #[test]
    fn search_response_roundtrip() {
        let r = SearchResponse {
            query: "memory".to_string(),
            results: vec![],
            total_found: 0,
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: SearchResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.query, "memory");
        assert_eq!(back.total_found, 0);
        assert!(back.results.is_empty());
    }

    // ---- PipelineHealthStatus ----

    #[test]
    fn pipeline_health_status_roundtrip() {
        let s = PipelineHealthStatus {
            regions_without_queries: 10,
            regions_with_queries: 1190,
            pending_fetch_tasks: 42,
            worker_count: 3,
        };
        let v = serde_json::to_value(&s).unwrap();
        let back: PipelineHealthStatus = serde_json::from_value(v).unwrap();
        assert_eq!(back.regions_with_queries, 1190);
        assert_eq!(back.pending_fetch_tasks, 42);
    }

    // ---- SummaryFreshness ----

    #[test]
    fn summary_freshness_default_is_zero() {
        let s = SummaryFreshness::default();
        assert_eq!(s.fresh, 0);
        assert_eq!(s.stale, 0);
        assert_eq!(s.no_summary, 0);
        assert_eq!(s.staleness_days, 0);
    }

    #[test]
    fn summary_freshness_roundtrip() {
        let s = SummaryFreshness {
            fresh: 800,
            stale: 300,
            no_summary: 100,
            staleness_days: 30,
        };
        let v = serde_json::to_value(&s).unwrap();
        let back: SummaryFreshness = serde_json::from_value(v).unwrap();
        assert_eq!(back.fresh, 800);
        assert_eq!(back.stale, 300);
        assert_eq!(back.no_summary, 100);
        assert_eq!(back.staleness_days, 30);
    }

    // ---- SystemStats / StatusCount / QueryDistEntry ----

    #[test]
    fn status_count_roundtrip() {
        let sc = StatusCount {
            status: "pending".to_string(),
            count: 99,
        };
        let v = serde_json::to_value(&sc).unwrap();
        let back: StatusCount = serde_json::from_value(v).unwrap();
        assert_eq!(back.status, "pending");
        assert_eq!(back.count, 99);
    }

    #[test]
    fn query_dist_entry_roundtrip() {
        let q = QueryDistEntry {
            query_count: 3,
            num_regions: 769,
        };
        let v = serde_json::to_value(&q).unwrap();
        let back: QueryDistEntry = serde_json::from_value(v).unwrap();
        assert_eq!(back.query_count, 3);
        assert_eq!(back.num_regions, 769);
    }

    #[test]
    fn system_stats_roundtrip() {
        let s = SystemStats {
            fetch_tasks_by_status: vec![StatusCount {
                status: "done".to_string(),
                count: 500,
            }],
            batches_by_status: vec![],
            total_queries: 2000,
            regions_with_queries: 1200,
            query_distribution: vec![QueryDistEntry {
                query_count: 3,
                num_regions: 700,
            }],
            total_papers: 10_000,
            total_summaries: 1_200,
            timestamp: ts(),
        };
        let v = serde_json::to_value(&s).unwrap();
        let back: SystemStats = serde_json::from_value(v).unwrap();
        assert_eq!(back.total_queries, 2000);
        assert_eq!(back.fetch_tasks_by_status.len(), 1);
        assert_eq!(back.query_distribution[0].num_regions, 700);
    }

    // ---- PipelineTriggerRequest / PipelineTriggerResult ----

    #[test]
    fn pipeline_trigger_request_empty_object_deserializes_to_defaults() {
        let r: PipelineTriggerRequest = serde_json::from_str("{}").unwrap();
        assert!(!r.reset_queries);
        assert!(!r.generate_queries);
        assert!(!r.discover_papers);
        assert!(!r.ensure_workers);
    }

    #[test]
    fn pipeline_trigger_request_default_matches_empty_object() {
        let a = PipelineTriggerRequest::default();
        let b: PipelineTriggerRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(a.reset_queries, b.reset_queries);
        assert_eq!(a.generate_queries, b.generate_queries);
        assert_eq!(a.discover_papers, b.discover_papers);
        assert_eq!(a.ensure_workers, b.ensure_workers);
    }

    #[test]
    fn pipeline_trigger_request_partial_payload() {
        let r: PipelineTriggerRequest =
            serde_json::from_str(r#"{"generate_queries":true,"ensure_workers":true}"#).unwrap();
        assert!(!r.reset_queries);
        assert!(r.generate_queries);
        assert!(!r.discover_papers);
        assert!(r.ensure_workers);
    }

    #[test]
    fn pipeline_trigger_result_default_has_empty_errors() {
        let r = PipelineTriggerResult::default();
        assert!(r.errors.is_empty());
        assert!(r.reset_queries_deleted.is_none());
        assert!(r.generate_queries_result.is_none());
        assert!(r.discover_papers_result.is_none());
        assert!(r.ensure_workers_ok.is_none());
    }

    #[test]
    fn pipeline_trigger_result_roundtrip_full() {
        let r = PipelineTriggerResult {
            reset_queries_deleted: Some(42),
            generate_queries_result: Some((10, 30)),
            discover_papers_result: Some((5, 12)),
            ensure_workers_ok: Some(true),
            errors: vec!["phase 2 failed on region X".to_string()],
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["reset_queries_deleted"], 42);
        // Tuple serializes to array.
        assert_eq!(v["generate_queries_result"][0], 10);
        assert_eq!(v["generate_queries_result"][1], 30);
        assert_eq!(v["ensure_workers_ok"], true);
        let back: PipelineTriggerResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.reset_queries_deleted, Some(42));
        assert_eq!(back.generate_queries_result, Some((10, 30)));
        assert_eq!(back.discover_papers_result, Some((5, 12)));
        assert_eq!(back.ensure_workers_ok, Some(true));
        assert_eq!(back.errors, vec!["phase 2 failed on region X".to_string()]);
    }

    // ---- RedisStats / RedisPrefixCount ----

    #[test]
    fn redis_prefix_count_roundtrip() {
        let p = RedisPrefixCount {
            pattern: "orch:region:*:status".to_string(),
            description: "region status cache".to_string(),
            count: 1198,
        };
        let v = serde_json::to_value(&p).unwrap();
        let back: RedisPrefixCount = serde_json::from_value(v).unwrap();
        assert_eq!(back.pattern, p.pattern);
        assert_eq!(back.count, 1198);
    }

    #[test]
    fn redis_stats_roundtrip_connected() {
        let s = RedisStats {
            connected: true,
            error: None,
            total_keys: 12345,
            keys_by_prefix: vec![RedisPrefixCount {
                pattern: "a".into(),
                description: "b".into(),
                count: 1,
            }],
            used_memory_bytes: 1024 * 1024 * 12,
            used_memory_human: "12.0M".to_string(),
            uptime_secs: 3600,
            total_connections_received: 42,
            keyspace_hits: 1000,
            keyspace_misses: 100,
            hit_rate: 0.909,
            server_version: "7.2.4".to_string(),
        };
        let v = serde_json::to_value(&s).unwrap();
        let back: RedisStats = serde_json::from_value(v).unwrap();
        assert!(back.connected);
        assert!(back.error.is_none());
        assert_eq!(back.total_keys, 12345);
        assert_eq!(back.keys_by_prefix.len(), 1);
        assert_eq!(back.server_version, "7.2.4");
    }

    #[test]
    fn redis_stats_roundtrip_disconnected() {
        let s = RedisStats {
            connected: false,
            error: Some("PING failed".to_string()),
            total_keys: 0,
            keys_by_prefix: vec![],
            used_memory_bytes: 0,
            used_memory_human: "0B".to_string(),
            uptime_secs: 0,
            total_connections_received: 0,
            keyspace_hits: 0,
            keyspace_misses: 0,
            hit_rate: 0.0,
            server_version: "".to_string(),
        };
        let v = serde_json::to_value(&s).unwrap();
        let back: RedisStats = serde_json::from_value(v).unwrap();
        assert!(!back.connected);
        assert_eq!(back.error.as_deref(), Some("PING failed"));
    }

    // ---- EvalRunCost ----

    #[test]
    fn eval_run_cost_roundtrip() {
        let c = EvalRunCost {
            run_id: "run-abc".to_string(),
            total_cost_usd: "0.012345".to_string(),
            total_input_tokens: 5000,
            total_output_tokens: 2500,
            total_calls: 7,
        };
        let v = serde_json::to_value(&c).unwrap();
        // Cost is a string to preserve precision.
        assert_eq!(v["total_cost_usd"], "0.012345");
        let back: EvalRunCost = serde_json::from_value(v).unwrap();
        assert_eq!(back.run_id, "run-abc");
        assert_eq!(back.total_cost_usd, "0.012345");
        assert_eq!(back.total_input_tokens, 5000);
        assert_eq!(back.total_output_tokens, 2500);
        assert_eq!(back.total_calls, 7);
    }
}
