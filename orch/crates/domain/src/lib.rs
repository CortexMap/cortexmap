use serde::{Deserialize, Serialize};
use uuid::Uuid;
use strum::{Display, EnumString, IntoStaticStr};

mod api_types;
mod batch_types;

pub use api_types::*;
pub use batch_types::*;

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
