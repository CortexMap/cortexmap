use super::{fetch_task_components, fetch_task_logs, fetch_tasks, papers};
use diesel::prelude::*;

// ==================== Papers ====================

/// Represents a new paper to be inserted into the database.
/// Used when creating a new record.
#[derive(Insertable, Debug)]
#[diesel(table_name = papers)]
pub struct NewPaper {
    pub pmc_id: String,
    pub s3_url: String,
    pub uid: String,
    pub query: String,
}

/// Represents a paper record retrieved from the database.
/// Includes all fields including the auto-generated id and timestamp.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = papers)]
pub struct Paper {
    pub id: i64,
    pub pmc_id: String,
    pub s3_url: String,
    pub uid: String,
    pub query: String,
    pub created_at: chrono::NaiveDateTime,
}

// ==================== Fetch Tasks ====================

/// Represents a new fetch task to be inserted into the database.
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = fetch_tasks)]
pub struct NewFetchTask {
    pub pmc_id: String,
    pub query: String,
    pub status: String,
    pub priority: i32,
}

/// Represents a fetch task record retrieved from the database.
/// Represents a task record retrieved from the database.
#[derive(Queryable, Selectable, QueryableByName, Debug, Clone, AsChangeset)]
#[diesel(table_name = fetch_tasks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FetchTask {
    pub id: i64,
    pub pmc_id: String,
    pub query: String,
    pub status: String,
    pub priority: i32,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
    pub started_at: Option<chrono::NaiveDateTime>,
    pub completed_at: Option<chrono::NaiveDateTime>,
    pub last_processed_at: Option<chrono::NaiveDateTime>,
    pub worker_id: Option<String>,
    pub heartbeat_at: Option<chrono::NaiveDateTime>,
    pub worker_version: Option<String>,
    pub stream_message_id: Option<String>,
}
// ==================== Fetch Task Components ====================

/// Represents a new task component to be inserted into the database.
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = fetch_task_components)]
pub struct NewFetchTaskComponent {
    pub task_id: i64,
    pub component_type: String,
    pub status: String,
    pub max_attempts: i32,
}

/// Represents a task component record retrieved from the database.
#[derive(Queryable, Selectable, Debug, Clone, AsChangeset)]
#[diesel(table_name = fetch_task_components)]
pub struct FetchTaskComponent {
    pub id: i64,
    pub task_id: i64,
    pub component_type: String,
    pub status: String,
    pub s3_key: Option<String>,
    pub attempt_count: i32,
    pub max_attempts: i32,
    pub error_message: Option<String>,
    pub last_attempted_at: Option<chrono::NaiveDateTime>,
    pub completed_at: Option<chrono::NaiveDateTime>,
}

// ==================== Fetch Task Logs ====================

/// Represents a new log entry to be inserted into the database.
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = fetch_task_logs)]
pub struct NewFetchTaskLog {
    pub task_id: i64,
    pub component_type: Option<String>,
    pub log_level: String,
    pub message: String,
    pub metadata: Option<serde_json::Value>,
}

/// Represents a log entry retrieved from the database.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = fetch_task_logs)]
pub struct FetchTaskLog {
    pub id: i64,
    pub task_id: i64,
    pub component_type: Option<String>,
    pub log_level: String,
    pub message: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: chrono::NaiveDateTime,
}
