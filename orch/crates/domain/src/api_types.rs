use serde::{Serialize, Deserialize};
use uuid::Uuid;

/// Priority levels for fetch/process tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Background,     // 0 - Routine scheduled scan
    Normal,         // 5 - Standard enqueue
    UserRequested,  // 8 - Triggered by search miss
    Invalidation,   // 10 - Force refresh
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
    NotStarted,   // No fetch task exists
    FetchQueued,  // Fetch task exists, status = pending
    Fetching,     // Fetch task status = in_progress
    FetchFailed,  // Fetch task status = failed
    LlmQueued,    // Fetch complete, handed to brainatlas, not done yet
    Processing,   // Brainatlas is chunking/embedding/summarizing
    Done,         // At least one region_summary entry exists
    Invalidated,  // New cycle queued on top of existing summaries
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
    pub summary: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// The batch that generated this summary
    pub batch_id: Uuid,
    /// Source chunks used to generate this summary
    pub sources: Vec<SummarySource>,
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
    /// The newly created batch ID
    pub batch_id: Uuid,
    /// Number of queries generated
    pub query_count: usize,
    /// Number of fetch tasks enqueued
    pub task_count: usize,
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

// ── Worker management types (mirrors proto/app/queue.proto:186-234) ──────────

/// Information about a single fetcher worker
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkerInfo {
    pub worker_id: String,
    pub status: String,
    #[serde(default)]
    pub current_task: String,
    #[serde(default)]
    pub tasks_processed: i64,
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub worker_version: String,
    #[serde(default)]
    pub last_heartbeat_at: i64,
    #[serde(default)]
    pub uptime_seconds: f64,
    #[serde(default)]
    pub tasks_failed: i64,
    #[serde(default)]
    pub success_rate: f64,
}

/// Response for GET /api/workers/status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatusResult {
    pub workers: Vec<WorkerInfo>,
}

/// Request body for POST /api/workers/allocate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocateWorkersRequest {
    pub worker_count: u32,
    pub task_timeout_secs: u64,
    pub max_retry_attempts: u32,
}

/// Response for POST /api/workers/allocate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocateWorkersResult {
    pub success: bool,
    pub worker_ids: Vec<String>,
    pub error_message: String,
}

/// Request body for POST /api/workers/stop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopWorkersRequest {
    /// Worker IDs to stop; empty slice means stop all workers
    pub worker_ids: Vec<String>,
}

/// Response for POST /api/workers/stop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopWorkersResult {
    pub success: bool,
    pub workers_stopped: u32,
    pub error_message: String,
}
