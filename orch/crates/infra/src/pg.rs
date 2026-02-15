use crate::models::{NewProcessedFetchTask as DbNewProcessedFetchTask, OrchConfig as DbOrchConfig, ProcessedFetchTask as DbProcessedFetchTask, UpdateOrchConfig};
use crate::InfraError;
use deadpool_diesel::postgres::{BuildError, Manager, Pool};
use deadpool_diesel::Runtime;
use diesel::prelude::*;
use services::{NewProcessedFetchTask, OrchConfig, OrchDatabase, ProcessedFetchTask};
use tokio::sync::OnceCell;

pub struct OrchPostgresql {
    pool: OnceCell<Pool>,
}

impl OrchPostgresql {
    pub fn new() -> Self {
        Self {
            pool: OnceCell::new(),
        }
    }

    /// Returns the cached pool, initialising it from `database_url` on the first call.
    async fn pool(&self, database_url: &str) -> Result<&Pool, BuildError> {
        self.pool
            .get_or_try_init(|| async {
                let manager = Manager::new(database_url, Runtime::Tokio1);
                Pool::builder(manager).max_size(10).build()
            })
            .await
    }
}

#[async_trait::async_trait]
impl OrchDatabase for OrchPostgresql {
    type Error = InfraError;
    
    async fn get_processed_task(
        &self,
        database_url: &str,
        fetch_task_id: i64,
    ) -> Result<Option<ProcessedFetchTask>, Self::Error> {
        use crate::schema::processed_fetch_tasks;
        
        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                processed_fetch_tasks::table
                    .find(fetch_task_id)
                    .first::<DbProcessedFetchTask>(c)
                    .optional()
            })
            .await??;
        Ok(result.map(Into::into))
    }
    
    async fn insert_processed_task(
        &self,
        database_url: &str,
        task: NewProcessedFetchTask,
    ) -> Result<(), Self::Error> {
        use crate::schema::processed_fetch_tasks;
        
        let db_task: DbNewProcessedFetchTask = task.into();
        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| {
            diesel::insert_into(processed_fetch_tasks::table)
                .values(&db_task)
                .execute(c)
        })
        .await??;
        Ok(())
    }
    
    async fn update_brainatlas_status(
        &self,
        database_url: &str,
        fetch_task_id: i64,
        status: &str,
        error: Option<String>,
    ) -> Result<(), Self::Error> {
        use crate::schema::processed_fetch_tasks;
        
        let status = status.to_string();
        let conn = self.pool(database_url).await?.get().await?;
        
        conn.interact(move |c| {
            match status.as_str() {
                "in_progress" => {
                    diesel::update(processed_fetch_tasks::table.find(fetch_task_id))
                        .set((
                            processed_fetch_tasks::brainatlas_status.eq(&status),
                            processed_fetch_tasks::brainatlas_started_at.eq(diesel::dsl::now),
                        ))
                        .execute(c)
                }
                "completed" => {
                    diesel::update(processed_fetch_tasks::table.find(fetch_task_id))
                        .set((
                            processed_fetch_tasks::brainatlas_status.eq(&status),
                            processed_fetch_tasks::brainatlas_completed_at.eq(diesel::dsl::now),
                        ))
                        .execute(c)
                }
                "failed" => {
                    diesel::update(processed_fetch_tasks::table.find(fetch_task_id))
                        .set((
                            processed_fetch_tasks::brainatlas_status.eq(&status),
                            processed_fetch_tasks::error_message.eq(error),
                        ))
                        .execute(c)
                }
                _ => {
                    diesel::update(processed_fetch_tasks::table.find(fetch_task_id))
                        .set(processed_fetch_tasks::brainatlas_status.eq(&status))
                        .execute(c)
                }
            }
        })
        .await??;
        Ok(())
    }
    
    async fn get_config(
        &self,
        database_url: &str,
        key: &str,
    ) -> Result<Option<String>, Self::Error> {
        use crate::schema::orch_config;
        
        let key = key.to_string();
        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                orch_config::table
                    .find(&key)
                    .select(orch_config::value)
                    .first::<String>(c)
                    .optional()
            })
            .await??;
        Ok(result)
    }
    
    async fn get_all_config(
        &self,
        database_url: &str,
    ) -> Result<Vec<OrchConfig>, Self::Error> {
        use crate::schema::orch_config;
        
        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                orch_config::table.load::<DbOrchConfig>(c)
            })
            .await??;
        Ok(result.into_iter().map(Into::into).collect())
    }
    
    async fn update_config(
        &self,
        database_url: &str,
        key: &str,
        value: &str,
    ) -> Result<(), Self::Error> {
        use crate::schema::orch_config;
        
        let key = key.to_string();
        let update = UpdateOrchConfig {
            value: value.to_string(),
            updated_at: chrono::Utc::now().naive_utc(),
        };
        
        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| {
            diesel::update(orch_config::table.find(&key))
                .set(&update)
                .execute(c)
        })
        .await??;
        Ok(())
    }
}
