use crate::blueprint::connections::{Connections, Fetcher};

pub struct Blueprint {
    pub fetcher: Fetcher,
    pub connections: Connections,
}
