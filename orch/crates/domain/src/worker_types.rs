use serde::{Deserialize, Serialize};

fn default_backoff_strategy() -> String {
    "constant".to_string()
}

/// Worker status and statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub worker_id: String,
    pub status: String,               // "running", "idle", "stopped"
    pub current_task: Option<String>, // PMC ID or None
    pub tasks_processed: i64,
    pub started_at: i64, // Unix timestamp
    pub worker_version: Option<String>,
    pub last_heartbeat_at: Option<i64>, // Unix timestamp
    pub uptime_seconds: f64,
    pub tasks_failed: i64,
    pub success_rate: f64, // 0.0 to 1.0
    pub task_timeout_secs: u64,
    pub failure_backoff_base_secs: u64,
    pub max_retry_attempts: u32,
    pub backoff_strategy: String,
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
    /// Reserved for the device-subscription follow-up (v2).
    /// When a future fetcher-be instance allocates workers on behalf of a
    /// specific device, orch can populate this field without any wire-format
    /// change. Must remain `None` in v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
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
    /// Reserved for the device-subscription follow-up (v2).
    /// Once fetcher-be starts reporting 429-driven cooldowns, orch needs to
    /// transmit the cooldown duration per device. Adding the optional field
    /// now avoids a v2 API break. Must remain `None`/unset in v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_cooldown_secs: Option<u64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetcher_retry_config_defaults_backoff_strategy_when_missing() {
        let config: FetcherRetryConfig = serde_json::from_str("{}").unwrap();

        assert_eq!(config.backoff_strategy, "constant");
        assert!(config.max_delay_secs.is_none());
        assert!(config.jitter.is_none());
        assert!(config.empty_queue_sleep_secs.is_none());
        assert!(config.stale_task_multiplier.is_none());
        assert!(config.summary_max_retries.is_none());
        assert!(config.abstract_max_retries.is_none());
        assert!(config.pdf_max_retries.is_none());
        assert!(config.device_cooldown_secs.is_none());
    }

    #[test]
    fn test_fetcher_retry_config_serialization_skips_none_fields() {
        let config = FetcherRetryConfig {
            backoff_strategy: "exponential".to_string(),
            max_delay_secs: Some(30),
            jitter: Some(0.25),
            empty_queue_sleep_secs: None,
            stale_task_multiplier: None,
            summary_max_retries: None,
            abstract_max_retries: Some(4),
            pdf_max_retries: None,
            device_cooldown_secs: None,
        };

        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["backoff_strategy"], "exponential");
        assert_eq!(json["max_delay_secs"], 30);
        assert_eq!(json["jitter"], 0.25);
        assert_eq!(json["abstract_max_retries"], 4);
        assert!(json.get("empty_queue_sleep_secs").is_none());
        assert!(json.get("stale_task_multiplier").is_none());
        assert!(json.get("summary_max_retries").is_none());
        assert!(json.get("pdf_max_retries").is_none());
        assert!(json.get("device_cooldown_secs").is_none());
    }

    #[test]
    fn test_allocate_workers_request_defaults_retry_config_to_none() {
        let request: AllocateWorkersRequest = serde_json::from_str(
            "{\"worker_count\":2,\"task_timeout_secs\":30,\"max_retry_attempts\":3}",
        )
        .unwrap();

        assert_eq!(request.worker_count, 2);
        assert_eq!(request.task_timeout_secs, 30);
        assert_eq!(request.max_retry_attempts, 3);
        assert!(request.retry_config.is_none());
        assert!(request.device_id.is_none());
    }

    #[test]
    fn test_allocate_workers_request_preserves_nested_retry_config() {
        let request: AllocateWorkersRequest = serde_json::from_str(
            "{\"worker_count\":4,\"task_timeout_secs\":45,\"max_retry_attempts\":5,\"retry_config\":{\"backoff_strategy\":\"linear\",\"max_delay_secs\":20,\"summary_max_retries\":7}}",
        )
        .unwrap();

        let retry = request.retry_config.unwrap();
        assert_eq!(retry.backoff_strategy, "linear");
        assert_eq!(retry.max_delay_secs, Some(20));
        assert_eq!(retry.summary_max_retries, Some(7));
        assert!(retry.abstract_max_retries.is_none());
        assert!(retry.pdf_max_retries.is_none());
        assert!(retry.device_cooldown_secs.is_none());
    }

    #[test]
    fn test_device_id_backward_compat_serialization() {
        // Verify device_id=None is not serialized (wire-compatible with existing fetcher-be)
        let request = AllocateWorkersRequest {
            worker_count: 2,
            task_timeout_secs: 30,
            max_retry_attempts: 3,
            retry_config: None,
            device_id: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("device_id").is_none());

        // Verify device_id=Some roundtrips correctly
        let request_with_device = AllocateWorkersRequest {
            worker_count: 2,
            task_timeout_secs: 30,
            max_retry_attempts: 3,
            retry_config: None,
            device_id: Some("device-abc".to_string()),
        };
        let json2 = serde_json::to_value(&request_with_device).unwrap();
        assert_eq!(json2["device_id"], "device-abc");
    }
}
