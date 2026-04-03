use cortexmap_infra::InfraError;

pub type RedisPool = deadpool_redis::Pool;

#[derive(Clone)]
pub struct StdRedisInfra {
    pool: RedisPool,
}

impl StdRedisInfra {
    pub fn new(redis_url: &str) -> Result<Self, InfraError> {
        let mut cfg = deadpool_redis::Config::from_url(redis_url);
        cfg.pool = Some(deadpool_redis::PoolConfig {
            max_size: 20,
            ..Default::default()
        });
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| InfraError::RedisError(e.to_string()))?;
        Ok(Self { pool })
    }

    pub async fn get_conn(&self) -> Result<deadpool_redis::Connection, InfraError> {
        self.pool
            .get()
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))
    }

    pub async fn bootstrap_queue(&self) -> Result<(), InfraError> {
        let mut conn = self.get_conn().await?;
        let result: Result<String, redis::RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg("fetcher:tasks")
            .arg("fetcher:workers")
            .arg("$")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("BUSYGROUP") => Ok(()),
            Err(e) => Err(InfraError::RedisError(e.to_string())),
        }
    }

    pub async fn ping(&self) -> Result<(), InfraError> {
        let mut conn = self.get_conn().await?;
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;
        Ok(())
    }

    pub async fn queue_pending_and_pel_count(&self) -> Result<(i64, i64), InfraError> {
        let mut conn = self.get_conn().await?;
        let xlen: i64 = redis::cmd("XLEN")
            .arg("fetcher:tasks")
            .query_async(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        let info: redis::Value = redis::cmd("XINFO")
            .arg("GROUPS")
            .arg("fetcher:tasks")
            .query_async(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        let pel_count = extract_pel_count(&info, "fetcher:workers").unwrap_or(0);
        Ok((xlen, pel_count))
    }
}

fn redis_value_as_str(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        redis::Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

fn extract_pel_count(value: &redis::Value, group_name: &str) -> Option<i64> {
    let groups = match value {
        redis::Value::Array(arr) => arr,
        _ => return None,
    };

    for group in groups {
        let fields = match group {
            redis::Value::Array(arr) => arr,
            _ => continue,
        };

        // fields is a flat list of alternating key-value pairs
        let mut i = 0;
        let mut found_group = false;
        let mut pel: i64 = 0;

        while i + 1 < fields.len() {
            if let Some(key) = redis_value_as_str(&fields[i]) {
                let val = &fields[i + 1];
                if key == "name" {
                    if let Some(name) = redis_value_as_str(val) {
                        found_group = name == group_name;
                    }
                } else if key == "pel-count" || key == "pending" {
                    if let redis::Value::Int(n) = val {
                        pel = *n;
                    }
                }
            }
            i += 2;
        }

        if found_group {
            return Some(pel);
        }
    }

    None
}
