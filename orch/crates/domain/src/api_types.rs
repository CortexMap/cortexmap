use uuid::Uuid;

/// Priority levels for fetch/process tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A single region summary entry
#[derive(Debug, Clone)]
pub struct RegionSummary {
    pub summary: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Result of searching for a region
#[derive(Debug, Clone)]
pub struct SearchRegionResult {
    pub status: RegionPipelineStatus,
    pub summaries: Vec<RegionSummary>,
}

/// Result of getting region status
#[derive(Debug, Clone)]
pub struct RegionStatusResult {
    pub region_id: Uuid,
    pub status: RegionPipelineStatus,
    pub last_fetch_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_summary_at: Option<chrono::DateTime<chrono::Utc>>,
    pub summary_count: i32,
    pub current_priority: Option<Priority>,
}

/// Result of invalidating a region
#[derive(Debug, Clone)]
pub struct InvalidateResult {
    pub region_id: Uuid,
    pub new_status: RegionPipelineStatus,
    pub detail: String,
}

/// Pipeline statistics across all regions
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub description: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Update for a configuration entry
#[derive(Debug, Clone)]
pub struct ConfigEntryUpdate {
    pub key: String,
    pub value: String,
}
