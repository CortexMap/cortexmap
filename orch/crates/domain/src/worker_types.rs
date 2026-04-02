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
    /// Retry configuration for task-level backoff and per-component limits
    #[serde(default)]
    pub retry_config: Option<FetcherRetryConfig>,
}

/// Retry configuration that orch forwards to the fetcher service.
/// Maps 1:1 to fetcher-be's `RetryConfig` + `ComponentRetryConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetcherRetryConfig {
    /// Backoff strategy: "constant", "linear", "exponential", or "fibonacci"
    #[serde(default = "default_backoff_strategy")]
    pub backoff_strategy: String,
    /// Maximum backoff delay in seconds (used by linear/exponential/fibonacci)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_delay_secs: Option<u64>,
    /// Jitter factor 0.0-1.0 (used by exponential)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter: Option<f64>,
    /// Sleep duration in seconds when queue is empty
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_queue_sleep_secs: Option<u64>,
    /// Multiplier for stale task detection (task_timeout_secs * multiplier)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_task_multiplier: Option<u64>,
    /// Per-component retry overrides (None = use global max_retry_attempts)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_max_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_max_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_max_retries: Option<u32>,
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
