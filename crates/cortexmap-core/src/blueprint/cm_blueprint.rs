use crate::blueprint::connections::{Connections, Fetcher};

#[derive(Debug, Clone)]
pub struct Blueprint {
    pub fetcher: Fetcher,
    pub connections: Connections,
}
