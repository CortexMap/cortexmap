use crate::models::{
    NewProcessedFetchTask as DbNewProcessedFetchTask, 
    OrchConfig as DbOrchConfig, 
    ProcessedFetchTask as DbProcessedFetchTask, 
    UpdateOrchConfig,
    RegionQueryRow,
    NewRegionQuery,
    ProcessingBatchRow,
    NewProcessingBatch,
};
use crate::InfraError;
use deadpool_diesel::postgres::{BuildError, Manager, Pool};
use deadpool_diesel::Runtime;
use diesel::prelude::*;
use services::{NewProcessedFetchTask, OrchConfig, OrchDatabase, ProcessedFetchTask, BatchManagement};
use domain::{RegionQuery, ProcessingBatch, BatchStatus};
use uuid::Uuid;
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
        key: domain::ConfigKey,
    ) -> Result<Option<String>, Self::Error> {
        use crate::schema::orch_config;
        
        let key_str = key.to_string();
        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                orch_config::table
                    .find(&key_str)
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
        key: domain::ConfigKey,
        value: &str,
    ) -> Result<(), Self::Error> {
        use crate::schema::orch_config;
        
        let key_str = key.to_string();
        let update = UpdateOrchConfig {
            value: value.to_string(),
            updated_at: chrono::Utc::now().naive_utc(),
        };
        
        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| {
            diesel::update(orch_config::table.find(&key_str))
                .set(&update)
                .execute(c)
        })
        .await??;
        Ok(())
    }
}

#[async_trait::async_trait]
impl BatchManagement for OrchPostgresql {
    type Error = InfraError;

    async fn get_queries(
        &self,
        database_url: &str,
        region_id: i32,
    ) -> Result<Vec<RegionQuery>, Self::Error> {
        use crate::schema::region_queries;
        
        let conn = self.pool(database_url).await?.get().await?;
        let rows = conn
            .interact(move |c| {
                region_queries::table
                    .filter(region_queries::region_id.eq(region_id))
                    .filter(region_queries::enabled.eq(true))
                    .order(region_queries::created_at.asc())
                    .load::<RegionQueryRow>(c)
            })
            .await??;
        
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn insert_queries(
        &self,
        database_url: &str,
        region_id: i32,
        queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error> {
        use crate::schema::region_queries;
        
        let new_queries: Vec<NewRegionQuery> = queries
            .into_iter()
            .map(|query_text| NewRegionQuery {
                region_id,
                query_text,
                source: "llm_generated".to_string(),
            })
            .collect();
        
        let conn = self.pool(database_url).await?.get().await?;
        let ids = conn
            .interact(move |c| {
                diesel::insert_into(region_queries::table)
                    .values(&new_queries)
                    .returning(region_queries::id)
                    .get_results::<Uuid>(c)
            })
            .await??;
        
        Ok(ids)
    }

    async fn create_batch(
        &self,
        database_url: &str,
        region_id: i32,
        expected_count: i32,
    ) -> Result<Uuid, Self::Error> {
        use crate::schema::region_processing_batches;
        
        let new_batch = NewProcessingBatch {
            region_id,
            expected_task_count: expected_count,
        };
        
        let conn = self.pool(database_url).await?.get().await?;
        let id = conn
            .interact(move |c| {
                diesel::insert_into(region_processing_batches::table)
                    .values(&new_batch)
                    .returning(region_processing_batches::id)
                    .get_result::<Uuid>(c)
            })
            .await??;
        
        Ok(id)
    }

    async fn add_tasks_to_batch(
        &self,
        database_url: &str,
        batch_id: Uuid,
        task_ids: Vec<i64>,
    ) -> Result<(), Self::Error> {
        use crate::schema::region_processing_batches;
        
        // Convert Vec<i64> to Vec<Option<i64>> for the array column
        let task_ids_opt: Vec<Option<i64>> = task_ids.into_iter().map(Some).collect();
        
        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| {
            diesel::update(region_processing_batches::table.find(batch_id))
                .set(region_processing_batches::fetch_task_ids.eq(task_ids_opt))
                .execute(c)
        })
        .await??;
        
        Ok(())
    }

    async fn get_batches_by_status(
        &self,
        database_url: &str,
        status: BatchStatus,
    ) -> Result<Vec<ProcessingBatch>, Self::Error> {
        use crate::schema::region_processing_batches;
        
        let status_str = status.as_str().to_string();
        
        let conn = self.pool(database_url).await?.get().await?;
        let rows = conn
            .interact(move |c| {
                region_processing_batches::table
                    .filter(region_processing_batches::status.eq(status_str))
                    .order(region_processing_batches::created_at.asc())
                    .load::<ProcessingBatchRow>(c)
            })
            .await??;
        
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_batch_status(
        &self,
        database_url: &str,
        batch_id: Uuid,
        status: BatchStatus,
        error: Option<String>,
    ) -> Result<(), Self::Error> {
        use crate::schema::region_processing_batches;
        use diesel::dsl::now;
        
        let status_str = status.as_str().to_string();
        
        let conn = self.pool(database_url).await?.get().await?;
        
        // Execute different updates based on status
        match (status, error) {
            (BatchStatus::Ready, None) => {
                conn.interact(move |c| {
                    diesel::update(region_processing_batches::table.find(batch_id))
                        .set((
                            region_processing_batches::status.eq(&status_str),
                            region_processing_batches::ready_at.eq(now),
                        ))
                        .execute(c)
                })
                .await??;
            }
            (BatchStatus::Processing, None) => {
                conn.interact(move |c| {
                    diesel::update(region_processing_batches::table.find(batch_id))
                        .set((
                            region_processing_batches::status.eq(&status_str),
                            region_processing_batches::processing_started_at.eq(now),
                        ))
                        .execute(c)
                })
                .await??;
            }
            (BatchStatus::Completed, None) | (BatchStatus::Failed, None) => {
                conn.interact(move |c| {
                    diesel::update(region_processing_batches::table.find(batch_id))
                        .set((
                            region_processing_batches::status.eq(&status_str),
                            region_processing_batches::completed_at.eq(now),
                        ))
                        .execute(c)
                })
                .await??;
            }
            (_, Some(err)) => {
                conn.interact(move |c| {
                    diesel::update(region_processing_batches::table.find(batch_id))
                        .set((
                            region_processing_batches::status.eq(&status_str),
                            region_processing_batches::error_message.eq(err),
                            region_processing_batches::completed_at.eq(now),
                        ))
                        .execute(c)
                })
                .await??;
            }
            _ => {
                conn.interact(move |c| {
                    diesel::update(region_processing_batches::table.find(batch_id))
                        .set(region_processing_batches::status.eq(&status_str))
                        .execute(c)
                })
                .await??;
            }
        }
        
        Ok(())
    }

    async fn complete_batch(
        &self,
        database_url: &str,
        batch_id: Uuid,
        summary_id: Uuid,
        content_hash: String,
    ) -> Result<(), Self::Error> {
        use crate::schema::region_processing_batches;
        use diesel::dsl::now;
        
        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| {
            diesel::update(region_processing_batches::table.find(batch_id))
                .set((
                    region_processing_batches::status.eq("completed"),
                    region_processing_batches::summary_id.eq(summary_id),
                    region_processing_batches::content_hash.eq(content_hash),
                    region_processing_batches::completed_at.eq(now),
                ))
                .execute(c)
        })
        .await??;
        
        Ok(())
    }

    async fn get_active_batch(
        &self,
        database_url: &str,
        region_id: i32,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        use crate::schema::region_processing_batches;
        
        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                region_processing_batches::table
                    .filter(region_processing_batches::region_id.eq(region_id))
                    .filter(region_processing_batches::status.eq_any(vec![
                        "collecting",
                        "ready",
                        "processing",
                    ]))
                    .first::<ProcessingBatchRow>(c)
                    .optional()
            })
            .await??;
        
        Ok(result.map(Into::into))
    }
}
