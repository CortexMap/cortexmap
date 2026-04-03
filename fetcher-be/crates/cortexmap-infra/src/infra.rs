use crate::error::InfraError;
use crate::{FetchTask, FetchTaskComponent, NewFetchTaskLog, NewPaper, Paper};
use bytes::Bytes;
use futures::Stream;
use reqwest::Response;
use std::fmt::{Display, Formatter};
use std::pin::Pin;

/// Environment variable access — collect vars once at startup and reuse.
pub trait EnvInfra: Send + Sync {
    #[allow(clippy::result_large_err)]
    fn get_env_var(&self, key: &str) -> Result<String, InfraError>;
}

#[derive(Debug, Clone, Copy)]
pub enum ContentType {
    Text,
    Pdf,
    Json,
    Markdown,
}

impl Display for ContentType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentType::Text => {
                write!(f, "text/plain")
            }
            ContentType::Pdf => {
                write!(f, "application/pdf")
            }
            ContentType::Json => {
                write!(f, "application/json")
            }
            ContentType::Markdown => {
                write!(f, "text/markdown")
            }
        }
    }
}

#[async_trait::async_trait]
pub trait HttpInfra {
    // Note: Currently returns reqwest::Response directly.
    // Could be wrapped in a custom type for better abstraction if needed.
    async fn get(&self, url: &str) -> Result<Response, InfraError>;
    async fn post(&self, url: &str, body: Option<Bytes>) -> Result<Response, InfraError>;
}

#[async_trait::async_trait]
pub trait DatabaseInfra {
    /// Insert a new paper into the database
    async fn insert_paper(&self, new_paper: NewPaper) -> Result<Paper, InfraError>;
}

#[async_trait::async_trait]
pub trait S3Infra {
    async fn put_s3(
        &self,
        key: &str,
        content_type: ContentType,
        content: Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>,
    ) -> Result<(), InfraError>;

    /// Download content from S3 by key
    async fn get_s3(&self, key: &str) -> Result<String, InfraError>;
}

/// Component types that can be fetched for a paper
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum ComponentType {
    Summary,
    Abstract,
    Pdf,
}

impl ComponentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ComponentType::Summary => "summary",
            ComponentType::Abstract => "abstract",
            ComponentType::Pdf => "pdf",
        }
    }
}

/// Task status in the queue
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
        }
    }
}

impl Display for TaskStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[async_trait::async_trait]
pub trait TaskQueueInfra {
    /// Enqueue a new fetch task for a PMC ID
    /// Creates the task and all component records (summary, abstract, pdf)
    async fn enqueue_task(
        &self,
        pmc_id: String,
        query: String,
        max_attempts: i32,
    ) -> Result<FetchTask, InfraError>;

    /// Get the next pending task respecting timeout
    /// Only returns tasks where last_processed_at is None or older than timeout_secs
    /// Uses FOR UPDATE SKIP LOCKED for concurrent worker safety
    async fn get_next_pending_task(
        &self,
        timeout_secs: u64,
    ) -> Result<Option<FetchTask>, InfraError>;

    /// Mark a task as started (in_progress status)
    async fn mark_task_started(&self, task_id: i64) -> Result<(), InfraError>;

    /// Mark a task as completed
    async fn mark_task_completed(&self, task_id: i64) -> Result<(), InfraError>;

    /// Mark a task as failed
    async fn mark_task_failed(&self, task_id: i64, error: String) -> Result<(), InfraError>;

    /// Get all pending components for a task
    async fn get_pending_components(
        &self,
        task_id: i64,
    ) -> Result<Vec<FetchTaskComponent>, InfraError>;

    /// Update component status after fetch attempt
    async fn update_component_status(
        &self,
        task_id: i64,
        component_type: ComponentType,
        status: TaskStatus,
        s3_key: Option<String>,
        error: Option<String>,
    ) -> Result<(), InfraError>;

    /// Increment component attempt counter
    /// Returns the new attempt count
    async fn increment_component_attempt(
        &self,
        task_id: i64,
        component_type: ComponentType,
    ) -> Result<i32, InfraError>;

    /// Check if all components for a task are completed
    async fn all_components_completed(&self, task_id: i64) -> Result<bool, InfraError>;

    /// Reset tasks stuck in 'in_progress' state
    /// Used for recovering from worker crashes
    async fn reset_stale_tasks(&self, timeout_secs: u64) -> Result<usize, InfraError>;

    /// Log a task event
    async fn log_task_event(&self, log: NewFetchTaskLog) -> Result<(), InfraError>;

    /// Get task statistics (pending, in_progress, completed, failed counts)
    async fn get_task_stats(&self) -> Result<TaskStats, InfraError>;

    /// Get detailed task statistics with breakdowns
    async fn get_detailed_task_stats(&self) -> Result<DetailedTaskStats, InfraError>;

    /// Get component-level statistics
    async fn get_component_stats(&self) -> Result<ComponentStats, InfraError>;

    /// Get recent tasks (limit: most recent N tasks)
    async fn get_recent_tasks(&self, limit: i64) -> Result<Vec<RecentTaskInfo>, InfraError>;

    /// Get task details by PMC ID
    async fn get_task_by_pmc_id(&self, pmc_id: &str) -> Result<Option<FetchTask>, InfraError>;

    /// Get task details by task ID
    async fn get_task_by_id(&self, task_id: i64) -> Result<Option<FetchTask>, InfraError>;

    /// Get tasks by status with limit
    async fn get_tasks_by_status(
        &self,
        status: &str,
        limit: i32,
    ) -> Result<Vec<FetchTask>, InfraError>;

    /// Get components for a specific task
    async fn get_task_components(
        &self,
        task_id: i64,
    ) -> Result<Vec<FetchTaskComponent>, InfraError>;

    // ==================== Worker Heartbeat Management ====================

    /// Claim a task and assign it to a worker
    /// Sets worker_id, initializes heartbeat, and marks as in_progress
    async fn claim_task_for_worker(
        &self,
        task_id: i64,
        worker_id: String,
        worker_version: Option<String>,
    ) -> Result<(), InfraError>;

    /// Update the heartbeat timestamp for a task
    /// Should be called periodically while processing to prevent stale task detection
    async fn update_task_heartbeat(&self, task_id: i64) -> Result<(), InfraError>;

    /// Release all tasks assigned to a specific worker
    /// Sets status back to 'pending' and clears worker_id/heartbeat
    /// Used for graceful shutdown
    async fn release_worker_tasks(&self, worker_id: String) -> Result<usize, InfraError>;

    /// Release a single task back to pending (used when processing fails/incomplete)
    /// Clears worker_id, heartbeat_at, started_at and sets status to 'pending'
    async fn release_task(&self, task_id: i64) -> Result<(), InfraError>;

    /// Release tasks with stale heartbeats (worker likely crashed)
    /// Tasks with heartbeat older than timeout_secs are reset to 'pending'
    async fn release_stale_tasks_by_heartbeat(
        &self,
        timeout_secs: u64,
    ) -> Result<usize, InfraError>;
}

/// Statistics about tasks in the queue
#[derive(Debug, Clone)]
pub struct TaskStats {
    pub pending: i64,
    pub in_progress: i64,
    pub completed: i64,
    pub failed: i64,
    pub total: i64,
}

/// Detailed statistics about tasks
#[derive(Debug, Clone)]
pub struct DetailedTaskStats {
    pub basic: TaskStats,
    pub tasks_with_errors: i64,
    pub tasks_pending_retry: i64,
    pub tasks_in_progress_over_5min: i64,
    pub average_completion_time_secs: f64,
    pub oldest_pending_task_age_secs: Option<i64>,
}

/// Component-level statistics
#[derive(Debug, Clone)]
pub struct ComponentStats {
    pub summary_completed: i64,
    pub abstract_completed: i64,
    pub pdf_completed: i64,
    pub summary_failed: i64,
    pub abstract_failed: i64,
    pub pdf_failed: i64,
    pub total_pending: i64,
}

/// Recent task information
#[derive(Debug, Clone)]
pub struct RecentTaskInfo {
    pub pmc_id: String,
    pub status: String,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
    pub worker_id: Option<String>,
    pub components_completed: i32,
    pub total_components: i32,
    pub summary_s3_key: Option<String>,
    pub abstract_s3_key: Option<String>,
}
