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
