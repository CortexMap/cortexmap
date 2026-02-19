use crate::InfraError;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use tokio::sync::OnceCell;

pub struct OrchRedis {
    conn: OnceCell<ConnectionManager>,
}

impl OrchRedis {
    pub fn new() -> Self {
        Self {
            conn: OnceCell::new(),
        }
    }

    /// Returns the cached connection manager, initialising it from `REDIS_URL`
    /// on the first call. Falls back to `redis://127.0.0.1:6379` if the env var
    /// is not set.
    async fn conn(&self) -> Result<ConnectionManager, InfraError> {
        let cm = self
            .conn
            .get_or_try_init(|| async {
                let url = std::env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
                let client = redis::Client::open(url)?;
                ConnectionManager::new(client).await
            })
            .await?;
        Ok(cm.clone())
    }
}

#[async_trait::async_trait]
impl services::CacheClient for OrchRedis {
    type Error = InfraError;

    async fn cache_get(&self, key: &str) -> Result<Option<String>, Self::Error> {
        let mut conn = self.conn().await?;
        let val: Option<String> = conn.get(key).await?;
        Ok(val)
    }

    async fn cache_set(&self, key: &str, value: &str, ttl_secs: u64) -> Result<(), Self::Error> {
        let mut conn = self.conn().await?;
        conn.set_ex(key, value, ttl_secs).await?;
        Ok(())
    }

    async fn cache_del(&self, key: &str) -> Result<(), Self::Error> {
        let mut conn = self.conn().await?;
        let _: () = conn.del(key).await?;
        Ok(())
    }

    async fn cache_del_pattern(&self, pattern: &str) -> Result<u64, Self::Error> {
        let mut conn = self.conn().await?;
        let mut deleted: u64 = 0;
        let mut cursor: u64 = 0;

        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await?;

            if !keys.is_empty() {
                let count: u64 = conn.del(&keys).await?;
                deleted += count;
            }

            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(deleted)
    }
}
