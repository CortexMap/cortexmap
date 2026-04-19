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
        let () = conn.set_ex(key, value, ttl_secs).await?;
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

    async fn cache_stats(&self) -> Result<domain::RedisStats, Self::Error> {
        // Connection-level errors are converted to a "down" snapshot so the
        // dashboard renders even when Redis is unavailable.
        let mut conn = match self.conn().await {
            Ok(c) => c,
            Err(e) => {
                return Ok(domain::RedisStats {
                    connected: false,
                    error: Some(e.to_string()),
                    total_keys: 0,
                    keys_by_prefix: vec![],
                    used_memory_bytes: 0,
                    used_memory_human: "0B".to_string(),
                    uptime_secs: 0,
                    total_connections_received: 0,
                    keyspace_hits: 0,
                    keyspace_misses: 0,
                    hit_rate: 0.0,
                    server_version: String::new(),
                });
            }
        };

        // INFO returns a multi-line `key:value` blob. Parse just what we need.
        let info: String = redis::cmd("INFO")
            .query_async(&mut conn)
            .await
            .unwrap_or_default();
        let info_get = |k: &str| -> String {
            info.lines()
                .find_map(|line| {
                    let line = line.trim();
                    line.strip_prefix(&format!("{k}:"))
                        .map(|v| v.trim().to_string())
                })
                .unwrap_or_default()
        };
        let info_u64 = |k: &str| -> u64 { info_get(k).parse().unwrap_or(0) };

        let total_keys: u64 = redis::cmd("DBSIZE")
            .query_async(&mut conn)
            .await
            .unwrap_or(0);

        // Count keys per known prefix using SCAN (non-blocking, O(N) over the
        // keyspace but bounded by COUNT batches). We keep this list aligned
        // with the patterns built in `services::cache_keys`.
        let prefixes: &[(&str, &str)] = &[
            ("orch:regions:all", "Full region_mapping list (10 min TTL)"),
            (
                "orch:pipeline:stats",
                "Cross-region pipeline statistics (15s TTL)",
            ),
            (
                "orch:region:*:summaries",
                "Per-region summaries (2 min TTL)",
            ),
            (
                "orch:region:*:status",
                "Per-region pipeline status (15s TTL)",
            ),
            ("orch:batch:*:status", "Per-batch status (15s TTL)"),
            ("orch:config:all", "Full orch_config snapshot (2 min TTL)"),
            (
                "orch:chunk:*:source",
                "Chunk source resolution, immutable (10 min TTL)",
            ),
            ("orch:batches:status:*", "Batches grouped by status"),
            ("orch:search:*", "Cached reverse-search results"),
        ];

        let mut keys_by_prefix = Vec::with_capacity(prefixes.len());
        for (pattern, description) in prefixes {
            let count = scan_count(&mut conn, pattern).await.unwrap_or(0);
            keys_by_prefix.push(domain::RedisPrefixCount {
                pattern: (*pattern).to_string(),
                description: (*description).to_string(),
                count,
            });
        }

        let hits = info_u64("keyspace_hits");
        let misses = info_u64("keyspace_misses");
        let hit_rate = if hits + misses > 0 {
            hits as f64 / (hits + misses) as f64
        } else {
            0.0
        };

        Ok(domain::RedisStats {
            connected: true,
            error: None,
            total_keys,
            keys_by_prefix,
            used_memory_bytes: info_u64("used_memory"),
            used_memory_human: info_get("used_memory_human"),
            uptime_secs: info_u64("uptime_in_seconds"),
            total_connections_received: info_u64("total_connections_received"),
            keyspace_hits: hits,
            keyspace_misses: misses,
            hit_rate,
            server_version: info_get("redis_version"),
        })
    }
}

/// Count keys matching a glob pattern using a non-blocking SCAN cursor.
async fn scan_count(conn: &mut ConnectionManager, pattern: &str) -> Result<u64, redis::RedisError> {
    let mut cursor: u64 = 0;
    let mut total: u64 = 0;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(500)
            .query_async(conn)
            .await?;
        total += keys.len() as u64;
        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }
    Ok(total)
}
