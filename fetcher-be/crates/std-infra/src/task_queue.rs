use crate::redis_infra::StdRedisInfra;
use cortexmap_infra::{
    ComponentType, FetchTask, FetchTaskComponent, InfraError, NewFetchTask, NewFetchTaskComponent,
    NewFetchTaskLog, TaskQueueInfra, TaskStats, TaskStatus,
};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};

pub type DbPool = Pool<ConnectionManager<PgConnection>>;

#[derive(Clone)]
pub struct StdTaskQueue {
    pool: DbPool,
}

impl StdTaskQueue {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Helper to run blocking database operations in tokio thread pool
    async fn run_blocking<F, T>(&self, f: F) -> Result<T, InfraError>
    where
        F: FnOnce(&mut PgConnection) -> Result<T, diesel::result::Error> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.pool.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<T, InfraError> {
            let mut conn = pool.get()?;
            let result = f(&mut conn);
            
            // Explicitly rollback if there was an error to reset connection state
            if result.is_err() {
                let _ = diesel::sql_query("ROLLBACK").execute(&mut conn);
            }
            
            Ok(result?)
        })
        .await??;
        
        Ok(result)
    }
}

#[async_trait::async_trait]
impl TaskQueueInfra for StdTaskQueue {
    async fn enqueue_task(
        &self,
        pmc_id: String,
        query: String,
        max_attempts: i32,
    ) -> Result<FetchTask, InfraError> {
        use cortexmap_infra::schema::{fetch_task_components, fetch_tasks};

        self.run_blocking(move |conn| {
            conn.transaction(|conn| {
                // Insert the task or update if exists (using ON CONFLICT DO UPDATE to always return a row)
                let task: FetchTask = diesel::insert_into(fetch_tasks::table)
                    .values(NewFetchTask {
                        pmc_id: pmc_id.clone(),
                        query: query.clone(),
                        status: TaskStatus::Pending.as_str().to_string(),
                        priority: 0,
                    })
                    .on_conflict((fetch_tasks::pmc_id, fetch_tasks::query))
                    .do_update()
                    .set(fetch_tasks::updated_at.eq(diesel::dsl::now))
                    .get_result(conn)?;

                // Create component records for this task if they don't exist
                let components = vec![
                    ComponentType::Summary,
                    ComponentType::Abstract,
                    ComponentType::Pdf,
                ];

                for component_type in components {
                    diesel::insert_into(fetch_task_components::table)
                        .values(NewFetchTaskComponent {
                            task_id: task.id,
                            component_type: component_type.as_str().to_string(),
                            status: TaskStatus::Pending.as_str().to_string(),
                            max_attempts,
                        })
                        .on_conflict((
                            fetch_task_components::task_id,
                            fetch_task_components::component_type,
                        ))
                        .do_nothing()
                        .execute(conn)?;
                }

                Ok(task)
            })
        })
        .await
    }

    async fn get_next_pending_task(
        &self,
        timeout_secs: u64,
        _worker_id: &str,
    ) -> Result<Option<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        self.run_blocking(move |conn| {
            conn.transaction(|conn| {
                // Use raw SQL for the timeout calculation and FOR UPDATE SKIP LOCKED
                // This query selects pending tasks that either:
                // 1. Have never been processed (last_processed_at IS NULL)
                // 2. Were processed more than timeout_secs ago
                let query = format!(
                    "SELECT * FROM fetch_tasks 
                     WHERE status = 'pending' 
                       AND (last_processed_at IS NULL 
                            OR last_processed_at < NOW() - INTERVAL '{} seconds')
                     ORDER BY priority DESC, created_at ASC 
                     LIMIT 1 
                     FOR UPDATE SKIP LOCKED",
                    timeout_secs
                );

                let task: Option<FetchTask> = diesel::sql_query(&query)
                    .load::<FetchTask>(conn)?
                    .into_iter()
                    .next();

                // Update last_processed_at if we got a task
                if let Some(ref task) = task {
                    diesel::update(fetch_tasks::table.find(task.id))
                        .set(fetch_tasks::last_processed_at.eq(diesel::dsl::now))
                        .execute(conn)?;
                }

                Ok(task)
            })
        })
        .await
    }

    async fn mark_task_started(&self, task_id: i64) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set((
                    fetch_tasks::status.eq(TaskStatus::InProgress.as_str()),
                    fetch_tasks::started_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    async fn mark_task_completed(&self, task_id: i64) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set((
                    fetch_tasks::status.eq(TaskStatus::Completed.as_str()),
                    fetch_tasks::completed_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    async fn mark_task_failed(&self, task_id: i64, error: String) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        // Log the error first
        self.log_task_event(NewFetchTaskLog {
            task_id,
            component_type: None,
            log_level: "error".to_string(),
            message: error,
            metadata: None,
        })
        .await?;

        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set(fetch_tasks::status.eq(TaskStatus::Failed.as_str()))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    async fn get_pending_components(
        &self,
        task_id: i64,
    ) -> Result<Vec<FetchTaskComponent>, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            fetch_task_components::table
                .filter(fetch_task_components::task_id.eq(task_id))
                .filter(fetch_task_components::status.eq(TaskStatus::Pending.as_str()))
                .load::<FetchTaskComponent>(conn)
                .map_err(|e| diesel::result::Error::from(e))
        })
        .await
    }

    async fn update_component_status(
        &self,
        task_id: i64,
        component_type: ComponentType,
        status: TaskStatus,
        s3_key: Option<String>,
        error: Option<String>,
    ) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            // Build the update based on status
            match status {
                TaskStatus::Completed => {
                    diesel::update(
                        fetch_task_components::table
                            .filter(fetch_task_components::task_id.eq(task_id))
                            .filter(
                                fetch_task_components::component_type.eq(component_type.as_str()),
                            ),
                    )
                    .set((
                        fetch_task_components::status.eq(TaskStatus::Completed.as_str()),
                        fetch_task_components::s3_key.eq(s3_key),
                        fetch_task_components::completed_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                }
                TaskStatus::Failed => {
                    diesel::update(
                        fetch_task_components::table
                            .filter(fetch_task_components::task_id.eq(task_id))
                            .filter(
                                fetch_task_components::component_type.eq(component_type.as_str()),
                            ),
                    )
                    .set((
                        fetch_task_components::status.eq(TaskStatus::Failed.as_str()),
                        fetch_task_components::error_message.eq(error),
                        fetch_task_components::last_attempted_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                }
                TaskStatus::Pending => {
                    diesel::update(
                        fetch_task_components::table
                            .filter(fetch_task_components::task_id.eq(task_id))
                            .filter(
                                fetch_task_components::component_type.eq(component_type.as_str()),
                            ),
                    )
                    .set((
                        fetch_task_components::status.eq(TaskStatus::Pending.as_str()),
                        fetch_task_components::last_attempted_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                }
                TaskStatus::InProgress => {
                    diesel::update(
                        fetch_task_components::table
                            .filter(fetch_task_components::task_id.eq(task_id))
                            .filter(
                                fetch_task_components::component_type.eq(component_type.as_str()),
                            ),
                    )
                    .set((
                        fetch_task_components::status.eq(TaskStatus::InProgress.as_str()),
                        fetch_task_components::last_attempted_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                }
            }

            Ok(())
        })
        .await
    }

    async fn increment_component_attempt(
        &self,
        task_id: i64,
        component_type: ComponentType,
    ) -> Result<i32, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            // Increment and return new value
            let component: FetchTaskComponent = diesel::update(
                fetch_task_components::table
                    .filter(fetch_task_components::task_id.eq(task_id))
                    .filter(fetch_task_components::component_type.eq(component_type.as_str())),
            )
            .set(
                fetch_task_components::attempt_count
                    .eq(fetch_task_components::attempt_count + 1),
            )
            .get_result(conn)?;

            Ok(component.attempt_count)
        })
        .await
    }

    async fn all_components_completed(&self, task_id: i64) -> Result<bool, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            let total_count: i64 = fetch_task_components::table
                .filter(fetch_task_components::task_id.eq(task_id))
                .count()
                .get_result(conn)?;

            let completed_count: i64 = fetch_task_components::table
                .filter(fetch_task_components::task_id.eq(task_id))
                .filter(fetch_task_components::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;

            Ok(total_count == completed_count && total_count > 0)
        })
        .await
    }

    async fn log_task_event(&self, log: NewFetchTaskLog) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_task_logs;

        self.run_blocking(move |conn| {
            diesel::insert_into(fetch_task_logs::table)
                .values(log)
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    async fn get_task_stats(&self) -> Result<TaskStats, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        self.run_blocking(move |conn| {
            let pending: i64 = fetch_tasks::table
                .filter(fetch_tasks::status.eq(TaskStatus::Pending.as_str()))
                .count()
                .get_result(conn)?;

            let in_progress: i64 = fetch_tasks::table
                .filter(fetch_tasks::status.eq(TaskStatus::InProgress.as_str()))
                .count()
                .get_result(conn)?;

            let completed: i64 = fetch_tasks::table
                .filter(fetch_tasks::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;

            let failed: i64 = fetch_tasks::table
                .filter(fetch_tasks::status.eq(TaskStatus::Failed.as_str()))
                .count()
                .get_result(conn)?;

            let total = pending + in_progress + completed + failed;

            Ok(TaskStats {
                pending,
                in_progress,
                completed,
                failed,
                total,
            })
        })
        .await
    }
    
    async fn get_detailed_task_stats(&self) -> Result<cortexmap_infra::DetailedTaskStats, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;
        use diesel::sql_types::{BigInt, Double, Nullable};
        
        let basic_stats = self.get_task_stats().await?;
        
        self.run_blocking(move |conn| {
            // Count tasks with at least one failed component
            #[derive(QueryableByName)]
            struct TaskErrorCount {
                #[diesel(sql_type = BigInt)]
                count: i64,
            }
            
            let tasks_with_errors: i64 = diesel::sql_query(
                "SELECT COUNT(DISTINCT task_id) as count FROM fetch_task_components WHERE error_message IS NOT NULL"
            )
            .get_result::<TaskErrorCount>(conn)
            .map(|r| r.count)
            .unwrap_or(0);
            
            // Count pending tasks with previous attempts
            let tasks_pending_retry: i64 = fetch_tasks::table
                .filter(fetch_tasks::status.eq(TaskStatus::Pending.as_str()))
                .filter(fetch_tasks::last_processed_at.is_not_null())
                .count()
                .get_result(conn)?;
            
            // Count in-progress tasks over 5 minutes old
            #[derive(QueryableByName)]
            struct TaskCount {
                #[diesel(sql_type = BigInt)]
                count: i64,
            }
            
            let tasks_in_progress_over_5min: i64 = diesel::sql_query(
                "SELECT COUNT(*) as count FROM fetch_tasks 
                 WHERE status = 'in_progress' 
                 AND started_at < NOW() - INTERVAL '5 minutes'"
            )
            .get_result::<TaskCount>(conn)
            .map(|r| r.count)
            .unwrap_or(0);
            
            // Calculate average completion time
            #[derive(QueryableByName)]
            #[diesel(check_for_backend(diesel::pg::Pg))]
            struct AvgTime {
                #[diesel(sql_type = Nullable<Double>)]
                avg_secs: Option<f64>,
            }
            
            let avg_completion: Option<f64> = diesel::sql_query(
                "SELECT AVG(EXTRACT(EPOCH FROM (completed_at - created_at))) as avg_secs 
                 FROM fetch_tasks 
                 WHERE status = 'completed' AND completed_at IS NOT NULL"
            )
            .get_result::<AvgTime>(conn)
            .ok()
            .and_then(|r| r.avg_secs);
            
            // Get oldest pending task age
            #[derive(QueryableByName)]
            #[diesel(check_for_backend(diesel::pg::Pg))]
            struct OldestAge {
                #[diesel(sql_type = Nullable<BigInt>)]
                age_secs: Option<i64>,
            }
            
            let oldest_pending_task_age: Option<i64> = diesel::sql_query(
                "SELECT EXTRACT(EPOCH FROM (NOW() - created_at))::BIGINT as age_secs 
                 FROM fetch_tasks 
                 WHERE status = 'pending' 
                 ORDER BY created_at ASC 
                 LIMIT 1"
            )
            .get_result::<OldestAge>(conn)
            .ok()
            .and_then(|r| r.age_secs);
            
            Ok(cortexmap_infra::DetailedTaskStats {
                basic: basic_stats,
                tasks_with_errors,
                tasks_pending_retry,
                tasks_in_progress_over_5min,
                average_completion_time_secs: avg_completion.unwrap_or(0.0),
                oldest_pending_task_age_secs: oldest_pending_task_age,
            })
        })
        .await
    }
    
    async fn get_component_stats(&self) -> Result<cortexmap_infra::ComponentStats, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;
        
        self.run_blocking(move |conn| {
            let summary_completed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("summary"))
                .filter(fetch_task_components::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;
            
            let abstract_completed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("abstract"))
                .filter(fetch_task_components::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;
            
            let pdf_completed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("pdf"))
                .filter(fetch_task_components::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;
            
            let summary_failed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("summary"))
                .filter(fetch_task_components::status.eq(TaskStatus::Failed.as_str()))
                .count()
                .get_result(conn)?;
            
            let abstract_failed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("abstract"))
                .filter(fetch_task_components::status.eq(TaskStatus::Failed.as_str()))
                .count()
                .get_result(conn)?;
            
            let pdf_failed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("pdf"))
                .filter(fetch_task_components::status.eq(TaskStatus::Failed.as_str()))
                .count()
                .get_result(conn)?;
            
            let total_pending: i64 = fetch_task_components::table
                .filter(fetch_task_components::status.eq(TaskStatus::Pending.as_str()))
                .count()
                .get_result(conn)?;
            
            Ok(cortexmap_infra::ComponentStats {
                summary_completed,
                abstract_completed,
                pdf_completed,
                summary_failed,
                abstract_failed,
                pdf_failed,
                total_pending,
            })
        })
        .await
    }
    
    async fn get_recent_tasks(&self, limit: i64) -> Result<Vec<cortexmap_infra::RecentTaskInfo>, InfraError> {
        use diesel::sql_types::{Text, BigInt, Timestamp, Integer, Nullable};
        
        self.run_blocking(move |conn| {
            #[derive(QueryableByName)]
            #[diesel(check_for_backend(diesel::pg::Pg))]
            struct RecentTaskRow {
                #[diesel(sql_type = Text)]
                pmc_id: String,
                #[diesel(sql_type = Text)]
                status: String,
                #[diesel(sql_type = Timestamp)]
                created_at: chrono::NaiveDateTime,
                #[diesel(sql_type = Timestamp)]
                updated_at: chrono::NaiveDateTime,
                #[diesel(sql_type = Nullable<Text>)]
                worker_id: Option<String>,
                #[diesel(sql_type = Integer)]
                components_completed: i32,
                #[diesel(sql_type = Integer)]
                total_components: i32,
                #[diesel(sql_type = Nullable<Text>)]
                summary_s3_key: Option<String>,
                #[diesel(sql_type = Nullable<Text>)]
                abstract_s3_key: Option<String>,
            }
            
            let results: Vec<RecentTaskRow> = diesel::sql_query(
                "SELECT 
                    t.pmc_id,
                    t.status,
                    t.created_at,
                    t.updated_at,
                    t.worker_id,
                    COALESCE((SELECT COUNT(*) FROM fetch_task_components c 
                              WHERE c.task_id = t.id AND c.status = 'completed'), 0)::INTEGER as components_completed,
                    COALESCE((SELECT COUNT(*) FROM fetch_task_components c 
                              WHERE c.task_id = t.id), 0)::INTEGER as total_components,
                    (SELECT s3_key FROM fetch_task_components c 
                     WHERE c.task_id = t.id AND c.component_type = 'summary' AND c.status = 'completed' LIMIT 1) as summary_s3_key,
                    (SELECT s3_key FROM fetch_task_components c 
                     WHERE c.task_id = t.id AND c.component_type = 'abstract' AND c.status = 'completed' LIMIT 1) as abstract_s3_key
                 FROM fetch_tasks t
                 ORDER BY t.updated_at DESC
                 LIMIT $1"
            )
            .bind::<BigInt, _>(limit)
            .load(conn)?;
            
            Ok(results.into_iter().map(|r| cortexmap_infra::RecentTaskInfo {
                pmc_id: r.pmc_id,
                status: r.status,
                created_at: r.created_at,
                updated_at: r.updated_at,
                worker_id: r.worker_id,
                components_completed: r.components_completed,
                total_components: r.total_components,
                summary_s3_key: r.summary_s3_key,
                abstract_s3_key: r.abstract_s3_key,
            }).collect())
        })
        .await
    }
    
    async fn get_task_by_pmc_id(&self, pmc_id: &str) -> Result<Option<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;
        
        let pmc_id = pmc_id.to_string();
        self.run_blocking(move |conn| {
            fetch_tasks::table
                .filter(fetch_tasks::pmc_id.eq(&pmc_id))
                .first::<FetchTask>(conn)
                .optional()
                .map_err(Into::into)
        })
        .await
    }
    
    async fn get_task_by_id(&self, task_id: i64) -> Result<Option<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;
        
        self.run_blocking(move |conn| {
            fetch_tasks::table
                .find(task_id)
                .first::<FetchTask>(conn)
                .optional()
                .map_err(Into::into)
        })
        .await
    }
    
    async fn get_tasks_by_status(&self, status: &str, limit: i32) -> Result<Vec<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;
        
        let status = status.to_string();
        self.run_blocking(move |conn| {
            fetch_tasks::table
                .filter(fetch_tasks::status.eq(&status))
                .order(fetch_tasks::completed_at.asc())
                .limit(limit as i64)
                .load::<FetchTask>(conn)
                .map_err(Into::into)
        })
        .await
    }
    
    async fn get_task_components(&self, task_id: i64) -> Result<Vec<FetchTaskComponent>, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;
        
        self.run_blocking(move |conn| {
            fetch_task_components::table
                .filter(fetch_task_components::task_id.eq(task_id))
                .load::<FetchTaskComponent>(conn)
                .map_err(Into::into)
        })
        .await
    }
    
    // ==================== Worker Heartbeat Management ====================
    
    async fn claim_task_for_worker(
        &self,
        task_id: i64,
        worker_id: String,
        worker_version: Option<String>,
    ) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;
        
        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set((
                    fetch_tasks::worker_id.eq(&worker_id),
                    fetch_tasks::heartbeat_at.eq(diesel::dsl::now),
                    fetch_tasks::worker_version.eq(worker_version),
                    fetch_tasks::status.eq(TaskStatus::InProgress.as_str()),
                    fetch_tasks::started_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            Ok(())
        })
        .await
    }
    
    async fn update_task_heartbeat(&self, task_id: i64) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set(fetch_tasks::heartbeat_at.eq(diesel::dsl::now))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    async fn release_task(&self, task_id: i64) -> Result<(), InfraError> {
        self.run_blocking(move |conn| {
            diesel::sql_query(
                "UPDATE fetch_tasks \
                 SET status = 'pending', \
                     worker_id = NULL, \
                     heartbeat_at = NULL, \
                     started_at = NULL \
                 WHERE id = $1",
            )
            .bind::<diesel::sql_types::BigInt, _>(task_id)
            .execute(conn)?;
            Ok(())
        })
        .await
    }

    async fn reclaim_stale_tasks(
        &self,
        _min_idle_ms: u64,
        _worker_id: &str,
    ) -> Result<Vec<FetchTask>, InfraError> {
        // Stub: full implementation provided by RedisTaskQueue
        Ok(vec![])
    }

    async fn update_task_heartbeat_redis(
        &self,
        _stream_id: &str,
        _ttl_secs: u64,
    ) -> Result<(), InfraError> {
        // Stub: full implementation provided by RedisTaskQueue
        Ok(())
    }
}


// ==================== RedisTaskQueue ====================

/// Task queue backed by Redis Streams for delivery and PostgreSQL for persistent state.
///
/// Key conventions:
/// - `fetcher:tasks`              — Redis Stream (primary task queue)
/// - `fetcher:workers`            — Consumer Group name on `fetcher:tasks`
/// - `fetcher:task:{task_id}`     — Hash (live state mirror)
/// - `fetcher:heartbeat:{stream_id}` — String+EX (worker liveness TTL)
#[derive(Clone)]
pub struct RedisTaskQueue {
    redis: StdRedisInfra,
    pg_pool: DbPool,
}

impl RedisTaskQueue {
    pub fn new(pg_pool: DbPool, redis: StdRedisInfra) -> Self {
        Self { redis, pg_pool }
    }

    /// Helper to run blocking database operations in tokio thread pool.
    /// Identical to `StdTaskQueue::run_blocking`.
    async fn run_blocking<F, T>(&self, f: F) -> Result<T, InfraError>
    where
        F: FnOnce(&mut PgConnection) -> Result<T, diesel::result::Error> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.pg_pool.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<T, InfraError> {
            let mut conn = pool.get()?;
            let result = f(&mut conn);

            // Explicitly rollback if there was an error to reset connection state
            if result.is_err() {
                let _ = diesel::sql_query("ROLLBACK").execute(&mut conn);
            }

            Ok(result?)
        })
        .await??;

        Ok(result)
    }

    /// Parse the response from `XREADGROUP GROUP … COUNT 1 BLOCK … STREAMS fetcher:tasks >`.
    ///
    /// Expected structure (redis 0.27 `Value`):
    /// ```text
    /// Array([
    ///   Array([
    ///     BulkString("fetcher:tasks"),
    ///     Array([
    ///       Array([BulkString(msg_id), Array([BulkString(k), BulkString(v), …])])
    ///     ])
    ///   ])
    /// ])
    /// ```
    /// Returns `None` on timeout (`Nil`), empty stream, or unexpected structure.
    fn parse_xreadgroup_reply(reply: redis::Value) -> Option<(String, i64)> {
        let streams = match reply {
            redis::Value::Array(v) if !v.is_empty() => v,
            _ => return None,
        };
        let stream_entry = match &streams[0] {
            redis::Value::Array(v) if v.len() >= 2 => v,
            _ => return None,
        };
        let messages = match &stream_entry[1] {
            redis::Value::Array(v) if !v.is_empty() => v,
            _ => return None,
        };
        let message = match &messages[0] {
            redis::Value::Array(v) if v.len() >= 2 => v,
            _ => return None,
        };
        let msg_id = match &message[0] {
            redis::Value::BulkString(b) => String::from_utf8(b.clone()).ok()?,
            _ => return None,
        };
        let fields = match &message[1] {
            redis::Value::Array(v) => v,
            _ => return None,
        };

        let task_id = Self::extract_task_id_from_fields(fields)?;
        Some((msg_id, task_id))
    }

    /// Parse messages returned by `XAUTOCLAIM`.
    ///
    /// Expected structure:
    /// ```text
    /// Array([
    ///   BulkString(next_cursor),
    ///   Array([Array([BulkString(msg_id), Array([fields…])]), …]),
    ///   Array([…deleted ids…])
    /// ])
    /// ```
    /// Returns a list of `(msg_id, task_id)` pairs for reclaimed messages.
    fn parse_xautoclaim_reply(reply: redis::Value) -> Vec<(String, i64)> {
        let parts = match reply {
            redis::Value::Array(v) if v.len() >= 2 => v,
            _ => return vec![],
        };
        let messages = match &parts[1] {
            redis::Value::Array(v) => v,
            _ => return vec![],
        };

        let mut result = Vec::new();
        for entry in messages {
            let entry_parts = match entry {
                redis::Value::Array(v) if v.len() >= 2 => v,
                _ => continue,
            };
            let msg_id = match &entry_parts[0] {
                redis::Value::BulkString(b) => match String::from_utf8(b.clone()) {
                    Ok(s) => s,
                    Err(_) => continue,
                },
                _ => continue,
            };
            let fields = match &entry_parts[1] {
                redis::Value::Array(v) => v,
                _ => continue,
            };
            if let Some(task_id) = Self::extract_task_id_from_fields(fields) {
                result.push((msg_id, task_id));
            }
        }
        result
    }

    /// Extract `task_id` (as `i64`) from a flat field-value list.
    fn extract_task_id_from_fields(fields: &[redis::Value]) -> Option<i64> {
        let mut i = 0;
        while i + 1 < fields.len() {
            let key = match &fields[i] {
                redis::Value::BulkString(b) => String::from_utf8(b.clone()).unwrap_or_default(),
                _ => {
                    i += 2;
                    continue;
                }
            };
            let val = match &fields[i + 1] {
                redis::Value::BulkString(b) => String::from_utf8(b.clone()).unwrap_or_default(),
                _ => {
                    i += 2;
                    continue;
                }
            };
            if key == "task_id" {
                return val.parse::<i64>().ok();
            }
            i += 2;
        }
        None
    }
}

#[async_trait::async_trait]
impl TaskQueueInfra for RedisTaskQueue {
    // ---- Task 2 ----
    async fn enqueue_task(
        &self,
        pmc_id: String,
        query: String,
        max_attempts: i32,
    ) -> Result<FetchTask, InfraError> {
        use cortexmap_infra::schema::{fetch_task_components, fetch_tasks};

        // Step 1: PG insert/upsert (verbatim from StdTaskQueue)
        let task: FetchTask = self
            .run_blocking(move |conn| {
                conn.transaction(|conn| {
                    let task: FetchTask = diesel::insert_into(fetch_tasks::table)
                        .values(NewFetchTask {
                            pmc_id: pmc_id.clone(),
                            query: query.clone(),
                            status: TaskStatus::Pending.as_str().to_string(),
                            priority: 0,
                        })
                        .on_conflict((fetch_tasks::pmc_id, fetch_tasks::query))
                        .do_update()
                        .set(fetch_tasks::updated_at.eq(diesel::dsl::now))
                        .get_result(conn)?;

                    let components = vec![
                        ComponentType::Summary,
                        ComponentType::Abstract,
                        ComponentType::Pdf,
                    ];
                    for component_type in components {
                        diesel::insert_into(fetch_task_components::table)
                            .values(NewFetchTaskComponent {
                                task_id: task.id,
                                component_type: component_type.as_str().to_string(),
                                status: TaskStatus::Pending.as_str().to_string(),
                                max_attempts,
                            })
                            .on_conflict((
                                fetch_task_components::task_id,
                                fetch_task_components::component_type,
                            ))
                            .do_nothing()
                            .execute(conn)?;
                    }
                    Ok(task)
                })
            })
            .await?;

        // Step 2: If already in Redis stream, return as-is (duplicate enqueue)
        if task.stream_message_id.is_some() {
            return Ok(task);
        }

        // Step 3 & 4: XADD + HSET
        let mut conn = self.redis.get_conn().await?;

        let stream_id: String = redis::cmd("XADD")
            .arg("fetcher:tasks")
            .arg("MAXLEN")
            .arg("~")
            .arg(10000u64)
            .arg("*")
            .arg("task_id")
            .arg(task.id.to_string())
            .arg("pmc_id")
            .arg(&task.pmc_id)
            .arg("query")
            .arg(&task.query)
            .arg("priority")
            .arg(task.priority.to_string())
            .arg("max_attempts")
            .arg(max_attempts.to_string())
            .query_async(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        redis::cmd("HSET")
            .arg(format!("fetcher:task:{}", task.id))
            .arg("stream_id")
            .arg(&stream_id)
            .arg("status")
            .arg("pending")
            .arg("pmc_id")
            .arg(&task.pmc_id)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        // Step 5: Update PG stream_message_id
        let task_id = task.id;
        let sid = stream_id.clone();
        self.run_blocking(move |conn| {
            diesel::sql_query("UPDATE fetch_tasks SET stream_message_id = $1 WHERE id = $2")
                .bind::<diesel::sql_types::Text, _>(&sid)
                .bind::<diesel::sql_types::BigInt, _>(task_id)
                .execute(conn)
                .map(|_| ())
                .map_err(diesel::result::Error::from)
        })
        .await?;

        // Step 6: Return task with stream_message_id set
        Ok(FetchTask {
            stream_message_id: Some(stream_id),
            ..task
        })
    }

    // ---- Task 3 ----
    async fn get_next_pending_task(
        &self,
        timeout_secs: u64,
        worker_id: &str,
    ) -> Result<Option<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        let mut conn = self.redis.get_conn().await?;
        let block_ms = timeout_secs * 1000;

        let reply: redis::Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg("fetcher:workers")
            .arg(worker_id)
            .arg("COUNT")
            .arg(1)
            .arg("BLOCK")
            .arg(block_ms)
            .arg("STREAMS")
            .arg("fetcher:tasks")
            .arg(">")
            .query_async(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        let (stream_id, task_id) = match Self::parse_xreadgroup_reply(reply) {
            Some(pair) => pair,
            None => return Ok(None),
        };

        // Load full FetchTask from PG
        let task_opt = self
            .run_blocking(move |conn| {
                fetch_tasks::table
                    .find(task_id)
                    .first::<FetchTask>(conn)
                    .optional()
                    .map_err(Into::into)
            })
            .await?;

        match task_opt {
            None => {
                // Task deleted from PG; clean up orphaned PEL entry
                let sid = stream_id.clone();
                redis::cmd("XACK")
                    .arg("fetcher:tasks")
                    .arg("fetcher:workers")
                    .arg(&sid)
                    .query_async::<redis::Value>(&mut conn)
                    .await
                    .map_err(|e| InfraError::RedisError(e.to_string()))?;
                Ok(None)
            }
            Some(task) => Ok(Some(FetchTask {
                stream_message_id: Some(stream_id),
                ..task
            })),
        }
    }

    // ---- Task 4 ----
    async fn claim_task_for_worker(
        &self,
        task_id: i64,
        worker_id: String,
        worker_version: Option<String>,
    ) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        let unix_now = chrono::Utc::now().timestamp();
        let version_str = worker_version.clone().unwrap_or_default();

        let mut conn = self.redis.get_conn().await?;
        redis::cmd("HSET")
            .arg(format!("fetcher:task:{}", task_id))
            .arg("status")
            .arg("in_progress")
            .arg("worker_id")
            .arg(&worker_id)
            .arg("worker_version")
            .arg(&version_str)
            .arg("started_at")
            .arg(unix_now)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        // PG UPDATE (verbatim from StdTaskQueue)
        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set((
                    fetch_tasks::worker_id.eq(&worker_id),
                    fetch_tasks::heartbeat_at.eq(diesel::dsl::now),
                    fetch_tasks::worker_version.eq(worker_version),
                    fetch_tasks::status.eq(TaskStatus::InProgress.as_str()),
                    fetch_tasks::started_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    // ---- Task 5 ----
    async fn update_task_heartbeat(&self, task_id: i64) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set(fetch_tasks::heartbeat_at.eq(diesel::dsl::now))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    // ---- Task 6 ----
    async fn update_task_heartbeat_redis(
        &self,
        stream_id: &str,
        ttl_secs: u64,
    ) -> Result<(), InfraError> {
        let mut conn = self.redis.get_conn().await?;
        redis::cmd("SET")
            .arg(format!("fetcher:heartbeat:{}", stream_id))
            .arg(1u8)
            .arg("EX")
            .arg(ttl_secs)
            .query_async::<redis::Value>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;
        Ok(())
    }

    // ---- Task 7 ----
    async fn mark_task_completed(&self, task_id: i64) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        let mut conn = self.redis.get_conn().await?;
        let unix_now = chrono::Utc::now().timestamp();

        // Step 1: Retrieve stream_id from Redis hash
        let stream_id: String = redis::cmd("HGET")
            .arg(format!("fetcher:task:{}", task_id))
            .arg("stream_id")
            .query_async(&mut conn)
            .await
            .unwrap_or_default();

        // Step 2: XACK if we have a stream_id
        if !stream_id.is_empty() {
            redis::cmd("XACK")
                .arg("fetcher:tasks")
                .arg("fetcher:workers")
                .arg(&stream_id)
                .query_async::<redis::Value>(&mut conn)
                .await
                .map_err(|e| InfraError::RedisError(e.to_string()))?;
        }

        // Step 3: Update hash status
        redis::cmd("HSET")
            .arg(format!("fetcher:task:{}", task_id))
            .arg("status")
            .arg("completed")
            .arg("completed_at")
            .arg(unix_now)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        // Step 4: Expire hash in 7 days
        redis::cmd("EXPIRE")
            .arg(format!("fetcher:task:{}", task_id))
            .arg(604800u64)
            .query_async::<redis::Value>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        // Step 5: Delete heartbeat key
        if !stream_id.is_empty() {
            redis::cmd("DEL")
                .arg(format!("fetcher:heartbeat:{}", stream_id))
                .query_async::<redis::Value>(&mut conn)
                .await
                .map_err(|e| InfraError::RedisError(e.to_string()))?;
        }

        // Step 6: PG UPDATE (verbatim from StdTaskQueue)
        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set((
                    fetch_tasks::status.eq(TaskStatus::Completed.as_str()),
                    fetch_tasks::completed_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    // ---- Task 8 ----
    async fn mark_task_failed(&self, task_id: i64, error: String) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        // Log the error first (verbatim from StdTaskQueue)
        self.log_task_event(NewFetchTaskLog {
            task_id,
            component_type: None,
            log_level: "error".to_string(),
            message: error.clone(),
            metadata: None,
        })
        .await?;

        let mut conn = self.redis.get_conn().await?;

        // Step 1: HGET stream_id
        let stream_id: String = redis::cmd("HGET")
            .arg(format!("fetcher:task:{}", task_id))
            .arg("stream_id")
            .query_async(&mut conn)
            .await
            .unwrap_or_default();

        // Step 2: XACK if non-empty
        if !stream_id.is_empty() {
            redis::cmd("XACK")
                .arg("fetcher:tasks")
                .arg("fetcher:workers")
                .arg(&stream_id)
                .query_async::<redis::Value>(&mut conn)
                .await
                .map_err(|e| InfraError::RedisError(e.to_string()))?;
        }

        // Step 3: HSET status + error + EXPIRE
        let error_truncated = if error.len() > 512 {
            error[..512].to_string()
        } else {
            error
        };
        redis::cmd("HSET")
            .arg(format!("fetcher:task:{}", task_id))
            .arg("status")
            .arg("failed")
            .arg("error")
            .arg(&error_truncated)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        redis::cmd("EXPIRE")
            .arg(format!("fetcher:task:{}", task_id))
            .arg(604800u64)
            .query_async::<redis::Value>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        // Step 4: DEL heartbeat
        if !stream_id.is_empty() {
            redis::cmd("DEL")
                .arg(format!("fetcher:heartbeat:{}", stream_id))
                .query_async::<redis::Value>(&mut conn)
                .await
                .map_err(|e| InfraError::RedisError(e.to_string()))?;
        }

        // Step 5: PG UPDATE (verbatim from StdTaskQueue)
        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set(fetch_tasks::status.eq(TaskStatus::Failed.as_str()))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    // ---- Task 9 ----
    async fn release_task(&self, task_id: i64) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        let mut conn = self.redis.get_conn().await?;

        // Step 1: HGET old stream_id
        let old_stream_id: String = redis::cmd("HGET")
            .arg(format!("fetcher:task:{}", task_id))
            .arg("stream_id")
            .query_async(&mut conn)
            .await
            .unwrap_or_default();

        // Step 2: Load task from PG for re-enqueue fields
        let task = self
            .run_blocking(move |conn| {
                fetch_tasks::table
                    .find(task_id)
                    .first::<FetchTask>(conn)
                    .map_err(Into::into)
            })
            .await?;

        // Step 2 (cont.): Get max_attempts from a component
        use cortexmap_infra::schema::fetch_task_components;
        let attempts = self
            .run_blocking(move |conn| {
                fetch_task_components::table
                    .filter(fetch_task_components::task_id.eq(task_id))
                    .select(fetch_task_components::max_attempts)
                    .first::<i32>(conn)
                    .optional()
                    .map(|opt| opt.unwrap_or(3))
                    .map_err(Into::into)
            })
            .await?;

        // Step 2 (cont.): XADD re-inject → new_stream_id
        let new_stream_id: String = redis::cmd("XADD")
            .arg("fetcher:tasks")
            .arg("MAXLEN")
            .arg("~")
            .arg(10000u64)
            .arg("*")
            .arg("task_id")
            .arg(task.id.to_string())
            .arg("pmc_id")
            .arg(&task.pmc_id)
            .arg("query")
            .arg(&task.query)
            .arg("priority")
            .arg(task.priority.to_string())
            .arg("max_attempts")
            .arg(attempts.to_string())
            .query_async(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        // Step 3: Update hash with new stream_id and reset state
        redis::cmd("HSET")
            .arg(format!("fetcher:task:{}", task_id))
            .arg("stream_id")
            .arg(&new_stream_id)
            .arg("status")
            .arg("pending")
            .arg("worker_id")
            .arg("")
            .arg("heartbeat_at")
            .arg("")
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| InfraError::RedisError(e.to_string()))?;

        // Step 4: XACK old PEL entry
        if !old_stream_id.is_empty() {
            redis::cmd("XACK")
                .arg("fetcher:tasks")
                .arg("fetcher:workers")
                .arg(&old_stream_id)
                .query_async::<redis::Value>(&mut conn)
                .await
                .map_err(|e| InfraError::RedisError(e.to_string()))?;
        }

        // Step 5: DEL old heartbeat key
        if !old_stream_id.is_empty() {
            redis::cmd("DEL")
                .arg(format!("fetcher:heartbeat:{}", old_stream_id))
                .query_async::<redis::Value>(&mut conn)
                .await
                .map_err(|e| InfraError::RedisError(e.to_string()))?;
        }

        // Step 6: PG UPDATE
        let new_sid = new_stream_id.clone();
        self.run_blocking(move |conn| {
            diesel::sql_query(
                "UPDATE fetch_tasks \
                 SET status = 'pending', \
                     worker_id = NULL, \
                     heartbeat_at = NULL, \
                     started_at = NULL, \
                     stream_message_id = $1 \
                 WHERE id = $2",
            )
            .bind::<diesel::sql_types::Text, _>(&new_sid)
            .bind::<diesel::sql_types::BigInt, _>(task_id)
            .execute(conn)
            .map(|_| ())
            .map_err(Into::into)
        })
        .await
    }

    // ---- Task 10 ----
    async fn reclaim_stale_tasks(
        &self,
        min_idle_ms: u64,
        worker_id: &str,
    ) -> Result<Vec<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        let mut conn = self.redis.get_conn().await?;
        let unix_now = chrono::Utc::now().timestamp();

        let reply: redis::Value = match redis::cmd("XAUTOCLAIM")
            .arg("fetcher:tasks")
            .arg("fetcher:workers")
            .arg(worker_id)
            .arg(min_idle_ms)
            .arg("0-0")
            .arg("COUNT")
            .arg(50u64)
            .query_async(&mut conn)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                // Stream or consumer group may not exist yet — return empty
                let msg = e.to_string();
                if msg.contains("NOGROUP") || msg.contains("ERR") {
                    return Ok(vec![]);
                }
                return Err(InfraError::RedisError(msg));
            }
        };

        let reclaimed = Self::parse_xautoclaim_reply(reply);
        if reclaimed.is_empty() {
            return Ok(vec![]);
        }

        let mut tasks = Vec::with_capacity(reclaimed.len());
        let worker_id_owned = worker_id.to_string();

        for (msg_id, task_id) in reclaimed {
            // Update Redis hash
            redis::cmd("HSET")
                .arg(format!("fetcher:task:{}", task_id))
                .arg("worker_id")
                .arg(&worker_id_owned)
                .arg("heartbeat_at")
                .arg(unix_now)
                .arg("status")
                .arg("in_progress")
                .query_async::<i64>(&mut conn)
                .await
                .map_err(|e| InfraError::RedisError(e.to_string()))?;

            // Load FetchTask from PG
            let task_opt = self
                .run_blocking(move |conn| {
                    fetch_tasks::table
                        .find(task_id)
                        .first::<FetchTask>(conn)
                        .optional()
                        .map_err(Into::into)
                })
                .await?;

            let task = match task_opt {
                Some(t) => t,
                None => continue,
            };

            // PG UPDATE
            let wid = worker_id_owned.clone();
            let sid = msg_id.clone();
            self.run_blocking(move |conn| {
                diesel::sql_query(
                    "UPDATE fetch_tasks \
                     SET worker_id = $1, \
                         heartbeat_at = NOW(), \
                         status = 'in_progress', \
                         stream_message_id = $2 \
                     WHERE id = $3",
                )
                .bind::<diesel::sql_types::Text, _>(&wid)
                .bind::<diesel::sql_types::Text, _>(&sid)
                .bind::<diesel::sql_types::BigInt, _>(task_id)
                .execute(conn)
                .map(|_| ())
                .map_err(Into::into)
            })
            .await?;

            tasks.push(FetchTask {
                stream_message_id: Some(msg_id),
                ..task
            });
        }

        Ok(tasks)
    }

    // ---- Task 11 ----
    async fn get_task_stats(&self) -> Result<TaskStats, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        // Try to get live pending/in_progress counts from Redis
        let (pending, in_progress) = match self.redis.queue_pending_and_pel_count().await {
            Ok((total_stream_len, pel_count)) => {
                let p = (total_stream_len - pel_count).max(0);
                (p, pel_count)
            }
            Err(e) => {
                tracing::warn!("Redis queue count failed, falling back to PG: {}", e);
                // Fall back to PG counts
                self.run_blocking(|conn| {
                    let p: i64 = fetch_tasks::table
                        .filter(fetch_tasks::status.eq(TaskStatus::Pending.as_str()))
                        .count()
                        .get_result(conn)?;
                    let ip: i64 = fetch_tasks::table
                        .filter(fetch_tasks::status.eq(TaskStatus::InProgress.as_str()))
                        .count()
                        .get_result(conn)?;
                    Ok((p, ip))
                })
                .await?
            }
        };

        // Completed and failed always come from PG
        let (completed, failed) = self
            .run_blocking(|conn| {
                let c: i64 = fetch_tasks::table
                    .filter(fetch_tasks::status.eq(TaskStatus::Completed.as_str()))
                    .count()
                    .get_result(conn)?;
                let f: i64 = fetch_tasks::table
                    .filter(fetch_tasks::status.eq(TaskStatus::Failed.as_str()))
                    .count()
                    .get_result(conn)?;
                Ok((c, f))
            })
            .await?;

        let total = pending + in_progress + completed + failed;
        Ok(TaskStats {
            pending,
            in_progress,
            completed,
            failed,
            total,
        })
    }

    // ---- Task 12 ----
    async fn get_detailed_task_stats(
        &self,
    ) -> Result<cortexmap_infra::DetailedTaskStats, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;
        use diesel::sql_types::{BigInt, Double, Nullable};

        // Use Redis-augmented stats for the basic counts
        let basic_stats = self.get_task_stats().await?;

        self.run_blocking(move |conn| {
            #[derive(QueryableByName)]
            struct TaskErrorCount {
                #[diesel(sql_type = BigInt)]
                count: i64,
            }

            let tasks_with_errors: i64 = diesel::sql_query(
                "SELECT COUNT(DISTINCT task_id) as count \
                 FROM fetch_task_components WHERE error_message IS NOT NULL",
            )
            .get_result::<TaskErrorCount>(conn)
            .map(|r| r.count)
            .unwrap_or(0);

            let tasks_pending_retry: i64 = fetch_tasks::table
                .filter(fetch_tasks::status.eq(TaskStatus::Pending.as_str()))
                .filter(fetch_tasks::last_processed_at.is_not_null())
                .count()
                .get_result(conn)?;

            #[derive(QueryableByName)]
            struct TaskCount {
                #[diesel(sql_type = BigInt)]
                count: i64,
            }

            let tasks_in_progress_over_5min: i64 = diesel::sql_query(
                "SELECT COUNT(*) as count FROM fetch_tasks \
                 WHERE status = 'in_progress' \
                 AND started_at < NOW() - INTERVAL '5 minutes'",
            )
            .get_result::<TaskCount>(conn)
            .map(|r| r.count)
            .unwrap_or(0);

            #[derive(QueryableByName)]
            #[diesel(check_for_backend(diesel::pg::Pg))]
            struct AvgTime {
                #[diesel(sql_type = Nullable<Double>)]
                avg_secs: Option<f64>,
            }

            let avg_completion: Option<f64> = diesel::sql_query(
                "SELECT AVG(EXTRACT(EPOCH FROM (completed_at - created_at))) as avg_secs \
                 FROM fetch_tasks \
                 WHERE status = 'completed' AND completed_at IS NOT NULL",
            )
            .get_result::<AvgTime>(conn)
            .ok()
            .and_then(|r| r.avg_secs);

            #[derive(QueryableByName)]
            #[diesel(check_for_backend(diesel::pg::Pg))]
            struct OldestAge {
                #[diesel(sql_type = Nullable<BigInt>)]
                age_secs: Option<i64>,
            }

            let oldest_pending_task_age: Option<i64> = diesel::sql_query(
                "SELECT EXTRACT(EPOCH FROM (NOW() - created_at))::BIGINT as age_secs \
                 FROM fetch_tasks \
                 WHERE status = 'pending' \
                 ORDER BY created_at ASC \
                 LIMIT 1",
            )
            .get_result::<OldestAge>(conn)
            .ok()
            .and_then(|r| r.age_secs);

            Ok(cortexmap_infra::DetailedTaskStats {
                basic: basic_stats,
                tasks_with_errors,
                tasks_pending_retry,
                tasks_in_progress_over_5min,
                average_completion_time_secs: avg_completion.unwrap_or(0.0),
                oldest_pending_task_age_secs: oldest_pending_task_age,
            })
        })
        .await
    }

    // ---- Task 13: remaining methods — verbatim copy from StdTaskQueue ----

    async fn mark_task_started(&self, task_id: i64) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        self.run_blocking(move |conn| {
            diesel::update(fetch_tasks::table.find(task_id))
                .set((
                    fetch_tasks::status.eq(TaskStatus::InProgress.as_str()),
                    fetch_tasks::started_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    async fn get_pending_components(
        &self,
        task_id: i64,
    ) -> Result<Vec<FetchTaskComponent>, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            fetch_task_components::table
                .filter(fetch_task_components::task_id.eq(task_id))
                .filter(fetch_task_components::status.eq(TaskStatus::Pending.as_str()))
                .load::<FetchTaskComponent>(conn)
                .map_err(|e| diesel::result::Error::from(e))
        })
        .await
    }

    async fn update_component_status(
        &self,
        task_id: i64,
        component_type: ComponentType,
        status: TaskStatus,
        s3_key: Option<String>,
        error: Option<String>,
    ) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            match status {
                TaskStatus::Completed => {
                    diesel::update(
                        fetch_task_components::table
                            .filter(fetch_task_components::task_id.eq(task_id))
                            .filter(
                                fetch_task_components::component_type.eq(component_type.as_str()),
                            ),
                    )
                    .set((
                        fetch_task_components::status.eq(TaskStatus::Completed.as_str()),
                        fetch_task_components::s3_key.eq(s3_key),
                        fetch_task_components::completed_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                }
                TaskStatus::Failed => {
                    diesel::update(
                        fetch_task_components::table
                            .filter(fetch_task_components::task_id.eq(task_id))
                            .filter(
                                fetch_task_components::component_type.eq(component_type.as_str()),
                            ),
                    )
                    .set((
                        fetch_task_components::status.eq(TaskStatus::Failed.as_str()),
                        fetch_task_components::error_message.eq(error),
                        fetch_task_components::last_attempted_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                }
                TaskStatus::Pending => {
                    diesel::update(
                        fetch_task_components::table
                            .filter(fetch_task_components::task_id.eq(task_id))
                            .filter(
                                fetch_task_components::component_type.eq(component_type.as_str()),
                            ),
                    )
                    .set((
                        fetch_task_components::status.eq(TaskStatus::Pending.as_str()),
                        fetch_task_components::last_attempted_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                }
                TaskStatus::InProgress => {
                    diesel::update(
                        fetch_task_components::table
                            .filter(fetch_task_components::task_id.eq(task_id))
                            .filter(
                                fetch_task_components::component_type.eq(component_type.as_str()),
                            ),
                    )
                    .set((
                        fetch_task_components::status.eq(TaskStatus::InProgress.as_str()),
                        fetch_task_components::last_attempted_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                }
            }
            Ok(())
        })
        .await
    }

    async fn increment_component_attempt(
        &self,
        task_id: i64,
        component_type: ComponentType,
    ) -> Result<i32, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            let component: FetchTaskComponent = diesel::update(
                fetch_task_components::table
                    .filter(fetch_task_components::task_id.eq(task_id))
                    .filter(fetch_task_components::component_type.eq(component_type.as_str())),
            )
            .set(
                fetch_task_components::attempt_count
                    .eq(fetch_task_components::attempt_count + 1),
            )
            .get_result(conn)?;
            Ok(component.attempt_count)
        })
        .await
    }

    async fn all_components_completed(&self, task_id: i64) -> Result<bool, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            let total_count: i64 = fetch_task_components::table
                .filter(fetch_task_components::task_id.eq(task_id))
                .count()
                .get_result(conn)?;

            let completed_count: i64 = fetch_task_components::table
                .filter(fetch_task_components::task_id.eq(task_id))
                .filter(fetch_task_components::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;

            Ok(total_count == completed_count && total_count > 0)
        })
        .await
    }

    async fn log_task_event(&self, log: NewFetchTaskLog) -> Result<(), InfraError> {
        use cortexmap_infra::schema::fetch_task_logs;

        self.run_blocking(move |conn| {
            diesel::insert_into(fetch_task_logs::table)
                .values(log)
                .execute(conn)?;
            Ok(())
        })
        .await
    }

    async fn get_component_stats(&self) -> Result<cortexmap_infra::ComponentStats, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            let summary_completed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("summary"))
                .filter(fetch_task_components::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;

            let abstract_completed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("abstract"))
                .filter(fetch_task_components::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;

            let pdf_completed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("pdf"))
                .filter(fetch_task_components::status.eq(TaskStatus::Completed.as_str()))
                .count()
                .get_result(conn)?;

            let summary_failed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("summary"))
                .filter(fetch_task_components::status.eq(TaskStatus::Failed.as_str()))
                .count()
                .get_result(conn)?;

            let abstract_failed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("abstract"))
                .filter(fetch_task_components::status.eq(TaskStatus::Failed.as_str()))
                .count()
                .get_result(conn)?;

            let pdf_failed: i64 = fetch_task_components::table
                .filter(fetch_task_components::component_type.eq("pdf"))
                .filter(fetch_task_components::status.eq(TaskStatus::Failed.as_str()))
                .count()
                .get_result(conn)?;

            let total_pending: i64 = fetch_task_components::table
                .filter(fetch_task_components::status.eq(TaskStatus::Pending.as_str()))
                .count()
                .get_result(conn)?;

            Ok(cortexmap_infra::ComponentStats {
                summary_completed,
                abstract_completed,
                pdf_completed,
                summary_failed,
                abstract_failed,
                pdf_failed,
                total_pending,
            })
        })
        .await
    }

    async fn get_recent_tasks(
        &self,
        limit: i64,
    ) -> Result<Vec<cortexmap_infra::RecentTaskInfo>, InfraError> {
        use diesel::sql_types::{BigInt, Integer, Nullable, Text, Timestamp};

        self.run_blocking(move |conn| {
            #[derive(QueryableByName)]
            #[diesel(check_for_backend(diesel::pg::Pg))]
            struct RecentTaskRow {
                #[diesel(sql_type = Text)]
                pmc_id: String,
                #[diesel(sql_type = Text)]
                status: String,
                #[diesel(sql_type = Timestamp)]
                created_at: chrono::NaiveDateTime,
                #[diesel(sql_type = Timestamp)]
                updated_at: chrono::NaiveDateTime,
                #[diesel(sql_type = Nullable<Text>)]
                worker_id: Option<String>,
                #[diesel(sql_type = Integer)]
                components_completed: i32,
                #[diesel(sql_type = Integer)]
                total_components: i32,
                #[diesel(sql_type = Nullable<Text>)]
                summary_s3_key: Option<String>,
                #[diesel(sql_type = Nullable<Text>)]
                abstract_s3_key: Option<String>,
            }

            let results: Vec<RecentTaskRow> = diesel::sql_query(
                "SELECT \
                    t.pmc_id, \
                    t.status, \
                    t.created_at, \
                    t.updated_at, \
                    t.worker_id, \
                    COALESCE((SELECT COUNT(*) FROM fetch_task_components c \
                              WHERE c.task_id = t.id AND c.status = 'completed'), 0)::INTEGER as components_completed, \
                    COALESCE((SELECT COUNT(*) FROM fetch_task_components c \
                              WHERE c.task_id = t.id), 0)::INTEGER as total_components, \
                    (SELECT s3_key FROM fetch_task_components c \
                     WHERE c.task_id = t.id AND c.component_type = 'summary' AND c.status = 'completed' LIMIT 1) as summary_s3_key, \
                    (SELECT s3_key FROM fetch_task_components c \
                     WHERE c.task_id = t.id AND c.component_type = 'abstract' AND c.status = 'completed' LIMIT 1) as abstract_s3_key \
                 FROM fetch_tasks t \
                 ORDER BY t.updated_at DESC \
                 LIMIT $1",
            )
            .bind::<BigInt, _>(limit)
            .load(conn)?;

            Ok(results
                .into_iter()
                .map(|r| cortexmap_infra::RecentTaskInfo {
                    pmc_id: r.pmc_id,
                    status: r.status,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                    worker_id: r.worker_id,
                    components_completed: r.components_completed,
                    total_components: r.total_components,
                    summary_s3_key: r.summary_s3_key,
                    abstract_s3_key: r.abstract_s3_key,
                })
                .collect())
        })
        .await
    }

    async fn get_task_by_pmc_id(&self, pmc_id: &str) -> Result<Option<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        let pmc_id = pmc_id.to_string();
        self.run_blocking(move |conn| {
            fetch_tasks::table
                .filter(fetch_tasks::pmc_id.eq(&pmc_id))
                .first::<FetchTask>(conn)
                .optional()
                .map_err(Into::into)
        })
        .await
    }

    async fn get_task_by_id(&self, task_id: i64) -> Result<Option<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        self.run_blocking(move |conn| {
            fetch_tasks::table
                .find(task_id)
                .first::<FetchTask>(conn)
                .optional()
                .map_err(Into::into)
        })
        .await
    }

    async fn get_tasks_by_status(
        &self,
        status: &str,
        limit: i32,
    ) -> Result<Vec<FetchTask>, InfraError> {
        use cortexmap_infra::schema::fetch_tasks;

        let status = status.to_string();
        self.run_blocking(move |conn| {
            fetch_tasks::table
                .filter(fetch_tasks::status.eq(&status))
                .order(fetch_tasks::completed_at.asc())
                .limit(limit as i64)
                .load::<FetchTask>(conn)
                .map_err(Into::into)
        })
        .await
    }

    async fn get_task_components(
        &self,
        task_id: i64,
    ) -> Result<Vec<FetchTaskComponent>, InfraError> {
        use cortexmap_infra::schema::fetch_task_components;

        self.run_blocking(move |conn| {
            fetch_task_components::table
                .filter(fetch_task_components::task_id.eq(task_id))
                .load::<FetchTaskComponent>(conn)
                .map_err(Into::into)
        })
        .await
    }
}
