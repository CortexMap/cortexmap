use crate::InfraError;
use crate::models::{
    NewProcessedFetchTask as DbNewProcessedFetchTask, NewProcessingBatch, NewRegionQuery,
    OrchConfig as DbOrchConfig, PaperMetadataRow, ProcessedFetchTask as DbProcessedFetchTask,
    ProcessingBatchRow, RegionQueryRow, RegionSummaryRow, UpdateOrchConfig,
};
use crate::schema::region_summary;
use deadpool_diesel::Runtime;
use deadpool_diesel::postgres::{BuildError, Manager, Pool};
use diesel::prelude::*;
use domain::{BatchStatus, ProcessingBatch, RegionQuery};
use services::{
    BatchManagement, NewProcessedFetchTask, OrchConfig, OrchDatabase, ProcessedFetchTask,
};
use tokio::sync::OnceCell;
use uuid::Uuid;

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

        conn.interact(move |c| match status.as_str() {
            "in_progress" => diesel::update(processed_fetch_tasks::table.find(fetch_task_id))
                .set((
                    processed_fetch_tasks::brainatlas_status.eq(&status),
                    processed_fetch_tasks::brainatlas_started_at.eq(diesel::dsl::now),
                ))
                .execute(c),
            "completed" => diesel::update(processed_fetch_tasks::table.find(fetch_task_id))
                .set((
                    processed_fetch_tasks::brainatlas_status.eq(&status),
                    processed_fetch_tasks::brainatlas_completed_at.eq(diesel::dsl::now),
                ))
                .execute(c),
            "failed" => diesel::update(processed_fetch_tasks::table.find(fetch_task_id))
                .set((
                    processed_fetch_tasks::brainatlas_status.eq(&status),
                    processed_fetch_tasks::error_message.eq(error),
                ))
                .execute(c),
            _ => diesel::update(processed_fetch_tasks::table.find(fetch_task_id))
                .set(processed_fetch_tasks::brainatlas_status.eq(&status))
                .execute(c),
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

    async fn get_all_config(&self, database_url: &str) -> Result<Vec<OrchConfig>, Self::Error> {
        use crate::schema::orch_config;

        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| orch_config::table.load::<DbOrchConfig>(c))
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
        region_id: Uuid,
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
        region_id: Uuid,
        queries: Vec<String>,
    ) -> Result<Vec<uuid::Uuid>, Self::Error> {
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

    async fn delete_queries(&self, database_url: &str, region_id: Uuid) -> Result<(), Self::Error> {
        use crate::schema::region_queries;

        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| {
            diesel::delete(region_queries::table)
                .filter(region_queries::region_id.eq(region_id))
                .execute(c)
        })
        .await??;

        Ok(())
    }

    async fn create_batch(
        &self,
        database_url: &str,
        region_id: Uuid,
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

    async fn update_batch_expected_count(
        &self,
        database_url: &str,
        batch_id: Uuid,
        expected_count: i32,
    ) -> Result<(), Self::Error> {
        use crate::schema::region_processing_batches;

        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| {
            diesel::update(region_processing_batches::table.find(batch_id))
                .set(region_processing_batches::expected_task_count.eq(expected_count))
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

    async fn complete_batch(&self, database_url: &str, batch_id: Uuid) -> Result<(), Self::Error> {
        use crate::schema::region_processing_batches;
        use diesel::dsl::now;

        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| {
            diesel::update(region_processing_batches::table.find(batch_id))
                .set((
                    region_processing_batches::status.eq("completed"),
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
        region_id: Uuid,
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
                        // Don't include 'invalidated' - it's not truly "active"
                    ]))
                    .first::<ProcessingBatchRow>(c)
                    .optional()
            })
            .await??;

        Ok(result.map(Into::into))
    }

    async fn get_recent_batch(
        &self,
        database_url: &str,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        use crate::schema::region_processing_batches;

        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                region_processing_batches::table
                    .filter(region_processing_batches::region_id.eq(region_id))
                    .order(region_processing_batches::created_at.desc())
                    .first::<ProcessingBatchRow>(c)
                    .optional()
            })
            .await??;

        Ok(result.map(Into::into))
    }

    async fn get_batch_by_id(
        &self,
        database_url: &str,
        batch_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        use crate::schema::region_processing_batches;

        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                region_processing_batches::table
                    .find(batch_id)
                    .first::<ProcessingBatchRow>(c)
                    .optional()
            })
            .await??;

        Ok(result.map(Into::into))
    }

    async fn count_completed_tasks(
        &self,
        database_url: &str,
        task_ids: &[i64],
    ) -> Result<usize, Self::Error> {
        use crate::schema::fetch_tasks::dsl;
        use diesel::prelude::*;

        let task_ids_vec = task_ids.to_vec();
        let conn = self.pool(database_url).await?.get().await?;

        let count = conn
            .interact(move |c| {
                dsl::fetch_tasks
                    .filter(dsl::id.eq_any(task_ids_vec))
                    .filter(dsl::status.eq("completed"))
                    .count()
                    .get_result::<i64>(c)
            })
            .await??;

        Ok(count as usize)
    }

    async fn get_task_s3_keys(
        &self,
        database_url: &str,
        task_ids: &[i64],
    ) -> Result<Vec<String>, Self::Error> {
        use crate::schema::fetch_task_components::dsl;
        use diesel::prelude::*;

        let task_ids_vec = task_ids.to_vec();
        let conn = self.pool(database_url).await?.get().await?;

        let s3_keys = conn
            .interact(move |c| {
                dsl::fetch_task_components
                    .filter(dsl::task_id.eq_any(task_ids_vec))
                    .filter(dsl::s3_key.is_not_null())
                    .select(dsl::s3_key)
                    .load::<Option<String>>(c)
            })
            .await??;

        Ok(s3_keys.into_iter().flatten().collect())
    }

    async fn get_task_paper_metadata(
        &self,
        database_url: &str,
        task_ids: &[i64],
    ) -> Result<Vec<services::PaperMetadataRecord>, Self::Error> {
        let task_ids_vec = task_ids.to_vec();
        let conn = self.pool(database_url).await?.get().await?;

        // Use raw SQL to JOIN fetch_tasks, fetch_task_components, and papers
        // to get s3_key -> (pmc_id, uid, query)
        let rows = conn
            .interact(move |c| {
                diesel::sql_query(
                    "SELECT DISTINCT
                        ftc.s3_key,
                        ft.pmc_id,
                        p.uid,
                        ft.query
                     FROM fetch_task_components ftc
                     INNER JOIN fetch_tasks ft ON ft.id = ftc.task_id
                     LEFT JOIN papers p ON p.pmc_id = ft.pmc_id
                     WHERE ftc.task_id = ANY($1)
                       AND ftc.s3_key IS NOT NULL"
                )
                .bind::<diesel::sql_types::Array<diesel::sql_types::BigInt>, _>(&task_ids_vec)
                .load::<PaperMetadataRow>(c)
            })
            .await??;

        Ok(rows
            .into_iter()
            .map(|r| services::PaperMetadataRecord {
                s3_key: r.s3_key,
                pmc_id: Some(r.pmc_id),
                uid: r.uid,
                query: Some(r.query),
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl services::RegionMappingQueries for OrchPostgresql {
    type Error = InfraError;

    async fn get_region_mapping(
        &self,
        database_url: &str,
        region_uuid: Uuid,
    ) -> Result<Option<services::RegionMapping>, Self::Error> {
        use crate::models::RegionMappingRow;
        use crate::schema::region_mapping;

        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                region_mapping::table
                    .find(region_uuid)
                    .first::<RegionMappingRow>(c)
                    .optional()
            })
            .await??;

        Ok(result.map(Into::into))
    }

    async fn get_all_regions(
        &self,
        database_url: &str,
    ) -> Result<Vec<services::RegionMapping>, Self::Error> {
        use crate::models::RegionMappingRow;
        use crate::schema::region_mapping;

        let conn = self.pool(database_url).await?.get().await?;
        let results = conn
            .interact(move |c| {
                region_mapping::table
                    .order(region_mapping::name.asc())
                    .load::<RegionMappingRow>(c)
            })
            .await??;

        Ok(results.into_iter().map(Into::into).collect())
    }

    async fn get_total_region_count(&self, database_url: &str) -> Result<i64, Self::Error> {
        use crate::schema::region_mapping;
        use diesel::dsl::count_star;

        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| region_mapping::table.select(count_star()).first::<i64>(c))
            .await??;

        Ok(result)
    }

    async fn count_regions_without_batches(&self, database_url: &str) -> Result<i64, Self::Error> {
        use crate::schema::{region_mapping, region_processing_batches};
        use diesel::dsl::{count_star, exists, not};

        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                region_mapping::table
                    .filter(not(exists(region_processing_batches::table.filter(
                        region_processing_batches::region_id.eq(region_mapping::id),
                    ))))
                    .select(count_star())
                    .first::<i64>(c)
            })
            .await??;

        Ok(result)
    }

    async fn get_region_summaries(
        &self,
        database_url: &str,
        region_id: i32,
    ) -> Result<Vec<services::RegionSummaryRecord>, Self::Error> {
        let conn = self.pool(database_url).await?.get().await?;
        let summaries = conn
            .interact(move |c| {
                region_summary::table
                    .filter(region_summary::region_id.eq(region_id))
                    .order(region_summary::created_at.desc())
                    .load::<RegionSummaryRow>(c)
            })
            .await??;

        Ok(summaries.into_iter().map(Into::into).collect())
    }

    async fn get_summary_sources(
        &self,
        database_url: &str,
        summary_id: Uuid,
    ) -> Result<Vec<services::ChunkSourceRecord>, Self::Error> {
        use crate::models::ChunkSourceRow;
        use crate::schema::brain_region_embeddings;

        let conn = self.pool(database_url).await?.get().await?;
        let rows = conn
            .interact(move |c| {
                brain_region_embeddings::table
                    .filter(brain_region_embeddings::summary_id.eq(summary_id))
                    .select(ChunkSourceRow::as_select())
                    .load::<ChunkSourceRow>(c)
            })
            .await??;

        Ok(rows.into_iter().map(Into::into).collect())
    }
}
