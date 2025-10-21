use crate::blueprint::Database;

pub struct Blueprint {
    // TODO: divide these into structs like
    // `Fetcher`, `Connections`, etc.
    pub query: String,
    pub page_size: u64,
    pub db: Database,
}
