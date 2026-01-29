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
            let result = f(&mut conn)?;
            Ok(result)
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
                // Insert the task (ON CONFLICT DO NOTHING for idempotency)
                let task: FetchTask = diesel::insert_into(fetch_tasks::table)
                    .values(NewFetchTask {
                        pmc_id: pmc_id.clone(),
                        query: query.clone(),
                        status: TaskStatus::Pending.as_str().to_string(),
                        priority: 0,
                    })
                    .on_conflict((fetch_tasks::pmc_id, fetch_tasks::query))
                    .do_nothing()
                    .get_result(conn)
                    .or_else(|_| {
                        // If conflict occurred, fetch the existing task
                        fetch_tasks::table
                            .filter(fetch_tasks::pmc_id.eq(&pmc_id))
                            .filter(fetch_tasks::query.eq(&query))
                            .first(conn)
                    })?;

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

    async fn reset_stale_tasks(&self, timeout_secs: u64) -> Result<usize, InfraError> {
        self.run_blocking(move |conn| {
            // Reset tasks that have been in_progress for more than timeout_secs * 3
            let stale_timeout = timeout_secs * 3;
            let query = format!(
                "UPDATE fetch_tasks 
                 SET status = 'pending', started_at = NULL 
                 WHERE status = 'in_progress' 
                   AND started_at < NOW() - INTERVAL '{} seconds'",
                stale_timeout
            );

            diesel::sql_query(&query)
                .execute(conn)
                .map_err(|e| diesel::result::Error::from(e))
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
    
    async fn release_worker_tasks(&self, worker_id: String) -> Result<usize, InfraError> {
        self.run_blocking(move |conn| {
            // Call the PostgreSQL function
            diesel::sql_query("SELECT release_worker_tasks($1)")
                .bind::<diesel::sql_types::Text, _>(&worker_id)
                .execute(conn)?;
            
            // Get the count by querying how many we just updated
            // (The function returns count but diesel doesn't support function return values easily)
            // So we just return 0 for now - the function does the work
            Ok(0)
        })
        .await
    }
    
    async fn release_stale_tasks_by_heartbeat(&self, timeout_secs: u64) -> Result<usize, InfraError> {
        let timeout = timeout_secs as i32;
        
        self.run_blocking(move |conn| {
            // Call the PostgreSQL function
            diesel::sql_query("SELECT release_stale_tasks($1)")
                .bind::<diesel::sql_types::Integer, _>(timeout)
                .execute(conn)?;
            
            // Same as above - function does the work, we return 0
            Ok(0)
        })
        .await
    }
}
