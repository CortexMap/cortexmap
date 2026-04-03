use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Source of a region query
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuerySource {
    LlmGenerated,
    UserAdded,
    UserModified,
}

impl QuerySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuerySource::LlmGenerated => "llm_generated",
            QuerySource::UserAdded => "user_added",
            QuerySource::UserModified => "user_modified",
        }
    }
}

impl From<&str> for QuerySource {
    fn from(s: &str) -> Self {
        match s {
            "user_added" => QuerySource::UserAdded,
            "user_modified" => QuerySource::UserModified,
            _ => QuerySource::LlmGenerated,
        }
    }
}

/// LLM-generated query for fetching papers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionQuery {
    pub id: Uuid,
    pub region_id: Uuid,
    pub query_text: String,
    pub source: QuerySource,
    pub priority: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Processing batch status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    /// Fetch tasks are in progress
    Collecting,
    /// All fetch tasks complete, ready to process
    Ready,
    /// Brainatlas is processing
    Processing,
    /// Summary generated successfully
    Completed,
    /// Processing failed
    Failed,
    /// Batch invalidated by user, will be recreated on next search
    Invalidated,
}

impl BatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BatchStatus::Collecting => "collecting",
            BatchStatus::Ready => "ready",
            BatchStatus::Processing => "processing",
            BatchStatus::Completed => "completed",
            BatchStatus::Failed => "failed",
            BatchStatus::Invalidated => "invalidated",
        }
    }
}

impl From<&str> for BatchStatus {
    fn from(s: &str) -> Self {
        match s {
            "ready" => BatchStatus::Ready,
            "processing" => BatchStatus::Processing,
            "completed" => BatchStatus::Completed,
            "failed" => BatchStatus::Failed,
            "invalidated" => BatchStatus::Invalidated,
            _ => BatchStatus::Collecting,
        }
    }
}

/// Processing batch tracking
/// Batch processing tracker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingBatch {
    pub id: Uuid,
    pub region_id: Uuid,
    pub status: BatchStatus,
    pub fetch_task_ids: Vec<i64>,
    pub expected_task_count: i32,
    pub content_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub ready_at: Option<DateTime<Utc>>,
    pub processing_started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub summary_id: Option<Uuid>,
    pub error_message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_source_as_str_matches_serialized_values() {
        assert_eq!(QuerySource::LlmGenerated.as_str(), "llm_generated");
        assert_eq!(QuerySource::UserAdded.as_str(), "user_added");
        assert_eq!(QuerySource::UserModified.as_str(), "user_modified");
    }

    #[test]
    fn test_query_source_from_str_defaults_unknown_values() {
        assert_eq!(QuerySource::from("user_added"), QuerySource::UserAdded);
        assert_eq!(
            QuerySource::from("user_modified"),
            QuerySource::UserModified
        );
        assert_eq!(
            QuerySource::from("anything_else"),
            QuerySource::LlmGenerated
        );
    }

    #[test]
    fn test_batch_status_as_str_matches_serialized_values() {
        assert_eq!(BatchStatus::Collecting.as_str(), "collecting");
        assert_eq!(BatchStatus::Ready.as_str(), "ready");
        assert_eq!(BatchStatus::Processing.as_str(), "processing");
        assert_eq!(BatchStatus::Completed.as_str(), "completed");
        assert_eq!(BatchStatus::Failed.as_str(), "failed");
        assert_eq!(BatchStatus::Invalidated.as_str(), "invalidated");
    }

    #[test]
    fn test_batch_status_from_str_defaults_unknown_values() {
        assert_eq!(BatchStatus::from("ready"), BatchStatus::Ready);
        assert_eq!(BatchStatus::from("processing"), BatchStatus::Processing);
        assert_eq!(BatchStatus::from("completed"), BatchStatus::Completed);
        assert_eq!(BatchStatus::from("failed"), BatchStatus::Failed);
        assert_eq!(BatchStatus::from("invalidated"), BatchStatus::Invalidated);
        assert_eq!(BatchStatus::from("something_else"), BatchStatus::Collecting);
    }
}
