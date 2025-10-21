use crate::blueprint::Database;

pub struct Blueprint {
    pub query: String,
    pub page_size: u64,
    pub db: Database,
}
