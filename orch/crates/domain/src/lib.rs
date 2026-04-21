use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr};
use uuid::Uuid;

mod api_types;
mod batch_types;
mod worker_types;

pub use api_types::*;
pub use batch_types::*;
pub use worker_types::*;

/// Configuration keys for orch_config table
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum ConfigKey {
    /// Interval in seconds between completion watcher polls
    CompletionPollIntervalSecs,
    /// Maximum number of tasks to process in parallel
    MaxParallelProcessCalls,
    /// Base URL for fetcher service
    FetcherBaseUrl,
    /// Base URL for brainatlas service
    BrainatlasBaseUrl,
    /// Number of search queries to generate per brain region
    QueryGenerationLimit,
    /// Default number of workers to allocate when tasks are enqueued
    DefaultWorkerCount,
    /// LLM model for embeddings (e.g., "text-embedding-3-small")
    EmbeddingModel,
    /// LLM model for chat/summarization (e.g., "openai/gpt-4o-mini")
    ChatModel,
    /// Maximum number of results returned by the reverse search endpoint
    SearchResultLimit,
    /// Default task timeout in seconds for fetcher workers
    FetcherTaskTimeoutSecs,
    /// Default max retry attempts for fetcher task components
    FetcherMaxRetryAttempts,
    /// Backoff strategy for fetcher retries: "constant", "linear", "exponential", "fibonacci"
    FetcherBackoffStrategy,
    /// Maximum backoff delay in seconds (for linear/exponential/fibonacci strategies)
    FetcherMaxDelaySecs,
    /// Jitter factor 0.0-1.0 for exponential backoff randomization
    FetcherBackoffJitter,
    /// Sleep duration in seconds when fetcher queue is empty
    FetcherEmptyQueueSleepSecs,
    /// Multiplier for stale task detection (task_timeout * multiplier)
    FetcherStaleTaskMultiplier,
    /// Max retry attempts specifically for summary component (overrides global)
    FetcherSummaryMaxRetries,
    /// Max retry attempts specifically for abstract component (overrides global)
    FetcherAbstractMaxRetries,
    /// Max retry attempts specifically for PDF component (overrides global)
    FetcherPdfMaxRetries,
    /// Sleep duration in seconds between pipeline cycles (default 3600 = 1 hour)
    PipelineCycleSleepSecs,
    /// Interval in seconds for the fast fetcher-monitor loop that re-probes worker
    /// health while the fetch queue is non-empty (default 30)
    FetcherMonitorIntervalSecs,
    /// Page size for NCBI ESearch enqueue requests in the pipeline's Phase 2 (default 20)
    EnqueuePageSize,
    /// Enable Phase-4 eval orchestrator background loop
    EvalOrchestratorEnabled,
    /// Poll interval (seconds) for the eval orchestrator
    EvalOrchestratorPollIntervalSecs,
    /// Max parallel `POST /evals-be/api/evals/score` calls
    EvalOrchestratorConcurrency,
    /// Base URL for evals-be (e.g. `http://evals-be:8083`)
    EvalsBaseUrl,
    /// Cache version passed to evals-be on every score request
    EvalVersion,
    /// Number of days a region's summary remains "fresh" before Phase 2 will
    /// re-process the region. Manual `/regions/:id/generate` ignores this gate.
    SummaryStalenessDays,
    /// Enable the LLM cost guardrail background loop.
    CostGuardrailEnabled,
    /// Poll interval (seconds) for the cost guardrail loop.
    CostGuardrailPollIntervalSecs,
    /// Max time in seconds a batch may stay in 'processing' before the
    /// stale-batch watcher marks it `failed` (default 1800 = 30 minutes).
    /// Recovers batches abandoned by a brainatlas-be crash/restart.
    ProcessingBatchTimeoutSecs,
}

/// Result of polling for completed fetch tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResult {
    /// Tasks that are ready to be processed
    pub tasks: Vec<PendingTask>,
    /// Total number of completed tasks found
    pub total_found: usize,
    /// Number of tasks that were already processed
    pub already_processed: usize,
}

/// A task pending LLM processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTask {
    pub task_id: i64,
    pub pmc_id: String,
    pub region_id: Uuid,
}

/// Result of processing tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessResult {
    /// Number of tasks successfully processed
    pub successful: usize,
    /// Number of tasks that failed
    pub failed: usize,
    /// Details of individual task results
    pub task_results: Vec<TaskResult>,
}

/// Result of processing a single task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: i64,
    pub pmc_id: String,
    pub region_id: Uuid,
    pub status: TaskStatus,
    pub detail: Option<String>,
}

/// Status of a processed task
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Success,
    Failed,
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // ---- ConfigKey (strum Display/FromStr, snake_case) ----

    /// Exhaustive list of every ConfigKey variant + its expected snake_case
    /// serialization. Uses a `match` so the compiler guarantees we cover every
    /// variant whenever a new key is added.
    fn config_key_cases() -> Vec<(ConfigKey, &'static str)> {
        use ConfigKey::*;
        let all = [
            CompletionPollIntervalSecs,
            MaxParallelProcessCalls,
            FetcherBaseUrl,
            BrainatlasBaseUrl,
            QueryGenerationLimit,
            DefaultWorkerCount,
            EmbeddingModel,
            ChatModel,
            SearchResultLimit,
            FetcherTaskTimeoutSecs,
            FetcherMaxRetryAttempts,
            FetcherBackoffStrategy,
            FetcherMaxDelaySecs,
            FetcherBackoffJitter,
            FetcherEmptyQueueSleepSecs,
            FetcherStaleTaskMultiplier,
            FetcherSummaryMaxRetries,
            FetcherAbstractMaxRetries,
            FetcherPdfMaxRetries,
            PipelineCycleSleepSecs,
            FetcherMonitorIntervalSecs,
            EnqueuePageSize,
            EvalOrchestratorEnabled,
            EvalOrchestratorPollIntervalSecs,
            EvalOrchestratorConcurrency,
            EvalsBaseUrl,
            EvalVersion,
            SummaryStalenessDays,
            CostGuardrailEnabled,
            CostGuardrailPollIntervalSecs,
            ProcessingBatchTimeoutSecs,
        ];
        all.into_iter()
            .map(|k| {
                let s: &'static str = match k {
                    CompletionPollIntervalSecs => "completion_poll_interval_secs",
                    MaxParallelProcessCalls => "max_parallel_process_calls",
                    FetcherBaseUrl => "fetcher_base_url",
                    BrainatlasBaseUrl => "brainatlas_base_url",
                    QueryGenerationLimit => "query_generation_limit",
                    DefaultWorkerCount => "default_worker_count",
                    EmbeddingModel => "embedding_model",
                    ChatModel => "chat_model",
                    SearchResultLimit => "search_result_limit",
                    FetcherTaskTimeoutSecs => "fetcher_task_timeout_secs",
                    FetcherMaxRetryAttempts => "fetcher_max_retry_attempts",
                    FetcherBackoffStrategy => "fetcher_backoff_strategy",
                    FetcherMaxDelaySecs => "fetcher_max_delay_secs",
                    FetcherBackoffJitter => "fetcher_backoff_jitter",
                    FetcherEmptyQueueSleepSecs => "fetcher_empty_queue_sleep_secs",
                    FetcherStaleTaskMultiplier => "fetcher_stale_task_multiplier",
                    FetcherSummaryMaxRetries => "fetcher_summary_max_retries",
                    FetcherAbstractMaxRetries => "fetcher_abstract_max_retries",
                    FetcherPdfMaxRetries => "fetcher_pdf_max_retries",
                    PipelineCycleSleepSecs => "pipeline_cycle_sleep_secs",
                    FetcherMonitorIntervalSecs => "fetcher_monitor_interval_secs",
                    EnqueuePageSize => "enqueue_page_size",
                    EvalOrchestratorEnabled => "eval_orchestrator_enabled",
                    EvalOrchestratorPollIntervalSecs => "eval_orchestrator_poll_interval_secs",
                    EvalOrchestratorConcurrency => "eval_orchestrator_concurrency",
                    EvalsBaseUrl => "evals_base_url",
                    EvalVersion => "eval_version",
                    SummaryStalenessDays => "summary_staleness_days",
                    CostGuardrailEnabled => "cost_guardrail_enabled",
                    CostGuardrailPollIntervalSecs => "cost_guardrail_poll_interval_secs",
                    ProcessingBatchTimeoutSecs => "processing_batch_timeout_secs",
                };
                (k, s)
            })
            .collect()
    }

    #[test]
    fn config_key_display_uses_snake_case() {
        for (key, expected) in config_key_cases() {
            assert_eq!(
                key.to_string(),
                expected,
                "unexpected Display for {:?}",
                key
            );
        }
    }

    #[test]
    fn config_key_into_static_str_uses_snake_case() {
        for (key, expected) in config_key_cases() {
            let s: &'static str = key.into();
            assert_eq!(s, expected);
        }
    }

    #[test]
    fn config_key_from_str_roundtrips_display() {
        for (key, wire) in config_key_cases() {
            let parsed = ConfigKey::from_str(wire)
                .unwrap_or_else(|_| panic!("failed to parse {wire:?} back to ConfigKey"));
            assert_eq!(parsed, key);
        }
    }

    #[test]
    fn config_key_from_str_rejects_unknown() {
        assert!(ConfigKey::from_str("not_a_real_key").is_err());
        assert!(ConfigKey::from_str("").is_err());
        // CamelCase variant names are not accepted — serialization is snake_case.
        assert!(ConfigKey::from_str("ChatModel").is_err());
    }

    // ---- PollResult ----

    #[test]
    fn poll_result_empty_roundtrip() {
        let r = PollResult {
            tasks: vec![],
            total_found: 0,
            already_processed: 0,
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: PollResult = serde_json::from_value(v).unwrap();
        assert!(back.tasks.is_empty());
        assert_eq!(back.total_found, 0);
        assert_eq!(back.already_processed, 0);
    }

    #[test]
    fn poll_result_with_tasks_roundtrip() {
        let region_id = Uuid::new_v4();
        let r = PollResult {
            tasks: vec![PendingTask {
                task_id: 101,
                pmc_id: "PMC555".to_string(),
                region_id,
            }],
            total_found: 5,
            already_processed: 4,
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: PollResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.tasks.len(), 1);
        assert_eq!(back.tasks[0].task_id, 101);
        assert_eq!(back.tasks[0].pmc_id, "PMC555");
        assert_eq!(back.tasks[0].region_id, region_id);
        assert_eq!(back.total_found, 5);
        assert_eq!(back.already_processed, 4);
    }

    // ---- PendingTask ----

    #[test]
    fn pending_task_roundtrip() {
        let t = PendingTask {
            task_id: 42,
            pmc_id: "PMC1".to_string(),
            region_id: Uuid::new_v4(),
        };
        let v = serde_json::to_value(&t).unwrap();
        let back: PendingTask = serde_json::from_value(v).unwrap();
        assert_eq!(back.task_id, t.task_id);
        assert_eq!(back.pmc_id, t.pmc_id);
        assert_eq!(back.region_id, t.region_id);
    }

    // ---- TaskStatus ----

    #[test]
    fn task_status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(TaskStatus::Success).unwrap(),
            "success"
        );
        assert_eq!(serde_json::to_value(TaskStatus::Failed).unwrap(), "failed");
        assert_eq!(
            serde_json::to_value(TaskStatus::Skipped).unwrap(),
            "skipped"
        );
    }

    #[test]
    fn task_status_deserializes_lowercase_wire() {
        let s: TaskStatus = serde_json::from_str("\"success\"").unwrap();
        assert!(matches!(s, TaskStatus::Success));
        let f: TaskStatus = serde_json::from_str("\"failed\"").unwrap();
        assert!(matches!(f, TaskStatus::Failed));
        let k: TaskStatus = serde_json::from_str("\"skipped\"").unwrap();
        assert!(matches!(k, TaskStatus::Skipped));
    }

    #[test]
    fn task_status_rejects_uppercase_wire() {
        // rename_all = "lowercase" means we expect exactly lowercase; any other case fails.
        assert!(serde_json::from_str::<TaskStatus>("\"Success\"").is_err());
        assert!(serde_json::from_str::<TaskStatus>("\"SUCCESS\"").is_err());
    }

    // ---- TaskResult ----

    #[test]
    fn task_result_roundtrip_with_detail() {
        let t = TaskResult {
            task_id: 10,
            pmc_id: "PMC10".to_string(),
            region_id: Uuid::new_v4(),
            status: TaskStatus::Skipped,
            detail: Some("already done".to_string()),
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["status"], "skipped");
        assert_eq!(v["detail"], "already done");
        let back: TaskResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.task_id, t.task_id);
        assert!(matches!(back.status, TaskStatus::Skipped));
        assert_eq!(back.detail.as_deref(), Some("already done"));
    }

    #[test]
    fn task_result_roundtrip_without_detail() {
        let t = TaskResult {
            task_id: 10,
            pmc_id: "PMC10".to_string(),
            region_id: Uuid::new_v4(),
            status: TaskStatus::Success,
            detail: None,
        };
        let v = serde_json::to_value(&t).unwrap();
        // detail has no skip_serializing_if → stays as null.
        assert!(v.get("detail").is_some());
        let back: TaskResult = serde_json::from_value(v).unwrap();
        assert!(back.detail.is_none());
        assert!(matches!(back.status, TaskStatus::Success));
    }

    // ---- ProcessResult ----

    #[test]
    fn process_result_roundtrip() {
        let region_id = Uuid::new_v4();
        let r = ProcessResult {
            successful: 7,
            failed: 2,
            task_results: vec![
                TaskResult {
                    task_id: 1,
                    pmc_id: "PMC1".to_string(),
                    region_id,
                    status: TaskStatus::Success,
                    detail: None,
                },
                TaskResult {
                    task_id: 2,
                    pmc_id: "PMC2".to_string(),
                    region_id,
                    status: TaskStatus::Failed,
                    detail: Some("timeout".to_string()),
                },
            ],
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: ProcessResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.successful, 7);
        assert_eq!(back.failed, 2);
        assert_eq!(back.task_results.len(), 2);
        assert!(matches!(back.task_results[0].status, TaskStatus::Success));
        assert!(matches!(back.task_results[1].status, TaskStatus::Failed));
        assert_eq!(back.task_results[1].detail.as_deref(), Some("timeout"));
    }
}
