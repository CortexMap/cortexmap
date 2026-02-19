use serde::{Deserialize, Serialize};

/// Worker status and statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub worker_id: String,
    pub status: String, // "running", "idle", "stopped"
    pub current_task: Option<String>, // PMC ID or None
    pub tasks_processed: i64,
    pub started_at: i64, // Unix timestamp
    pub worker_version: Option<String>,
    pub last_heartbeat_at: Option<i64>, // Unix timestamp
    pub uptime_seconds: f64,
    pub tasks_failed: i64,
    pub success_rate: f64, // 0.0 to 1.0
}

/// Request to allocate workers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocateWorkersRequest {
    pub worker_count: u32,
    pub task_timeout_secs: u64,
    pub max_retry_attempts: u32,
}

/// Response after allocating workers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerAllocationResponse {
    pub success: bool,
    pub worker_ids: Vec<String>,
    pub error_message: Option<String>,
}

/// Request to stop workers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopWorkersRequest {
    /// Specific worker IDs to stop. Empty list means stop all workers.
    pub worker_ids: Vec<String>,
}

/// Response after stopping workers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStopResponse {
    pub success: bool,
    pub workers_stopped: u32,
    pub error_message: Option<String>,
}
