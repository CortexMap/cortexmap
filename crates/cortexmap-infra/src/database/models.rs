use super::papers;
use diesel::prelude::*;

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
