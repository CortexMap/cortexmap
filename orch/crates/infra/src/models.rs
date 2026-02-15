use chrono::NaiveDateTime;
use diesel::prelude::*;
use uuid::Uuid;

// Diesel models for database tables

#[derive(Queryable, Selectable, Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::processed_fetch_tasks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProcessedFetchTask {
    pub fetch_task_id: i64,
    pub region_id: Uuid,
    pub pmc_id: String,
    pub processed_at: NaiveDateTime,
    pub brainatlas_status: String,
    pub brainatlas_started_at: Option<NaiveDateTime>,
    pub brainatlas_completed_at: Option<NaiveDateTime>,
    pub error_message: Option<String>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::processed_fetch_tasks)]
pub struct NewProcessedFetchTask {
    pub fetch_task_id: i64,
    pub region_id: Uuid,
    pub pmc_id: String,
    pub brainatlas_status: String,
}

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::orch_config)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrchConfig {
    pub key: String,
    pub value: String,
    pub description: Option<String>,
    pub updated_at: NaiveDateTime,
}

#[derive(AsChangeset, Debug)]
#[diesel(table_name = crate::schema::orch_config)]
pub struct UpdateOrchConfig {
    pub value: String,
    pub updated_at: NaiveDateTime,
}
