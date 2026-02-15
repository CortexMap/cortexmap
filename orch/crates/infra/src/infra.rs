use crate::InfraError;
use crate::env::OrchEnvInfra;
use crate::http::OrchHttpClient;
use crate::pg::OrchPostgresql;
use serde::Serialize;
use serde::de::DeserializeOwned;
use services::{
    EnvInfra, HttpClient, NewProcessedFetchTask, OrchConfig, OrchDatabase, ProcessedFetchTask,
    BatchManagement,
};
use domain::{RegionQuery, ProcessingBatch, BatchStatus};
use uuid::Uuid;

pub struct OrchInfra {
    env: OrchEnvInfra,
    pg: OrchPostgresql,
    http: OrchHttpClient,
}

impl OrchInfra {
    pub fn new() -> Self {
        Self {
            env: OrchEnvInfra::new(),
            pg: OrchPostgresql::new(),
            http: OrchHttpClient::new(),
        }
    }
}

impl EnvInfra for OrchInfra {
    type Error = InfraError;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
        self.env.get_env_var(key)
    }
}

#[async_trait::async_trait]
impl OrchDatabase for OrchInfra {
    type Error = InfraError;

    async fn get_processed_task(
        &self,
        database_url: &str,
        fetch_task_id: i64,
    ) -> Result<Option<ProcessedFetchTask>, Self::Error> {
        self.pg
            .get_processed_task(database_url, fetch_task_id)
            .await
    }

    async fn insert_processed_task(
        &self,
        database_url: &str,
        task: NewProcessedFetchTask,
    ) -> Result<(), Self::Error> {
        self.pg.insert_processed_task(database_url, task).await
    }

    async fn update_brainatlas_status(
        &self,
        database_url: &str,
        fetch_task_id: i64,
        status: &str,
        error: Option<String>,
    ) -> Result<(), Self::Error> {
        self.pg
            .update_brainatlas_status(database_url, fetch_task_id, status, error)
            .await
    }

    async fn get_config(
        &self,
        database_url: &str,
        key: domain::ConfigKey,
    ) -> Result<Option<String>, Self::Error> {
        self.pg.get_config(database_url, key).await
    }

    async fn get_all_config(&self, database_url: &str) -> Result<Vec<OrchConfig>, Self::Error> {
        self.pg.get_all_config(database_url).await
    }

    async fn update_config(
        &self,
        database_url: &str,
        key: domain::ConfigKey,
        value: &str,
    ) -> Result<(), Self::Error> {
        self.pg.update_config(database_url, key, value).await
    }
}

#[async_trait::async_trait]
impl HttpClient for OrchInfra {
    type Error = InfraError;

    async fn get<T: DeserializeOwned + Send>(&self, url: &str) -> Result<T, Self::Error> {
        self.http.get(url).await
    }

    async fn post<Req: Serialize + Send + Sync, Res: DeserializeOwned + Send + Sync>(
        &self,
        url: &str,
        body: &Req,
    ) -> Result<Res, Self::Error> {
        self.http.post(url, body).await
    }
}

#[async_trait::async_trait]
impl BatchManagement for OrchInfra {
    type Error = InfraError;

    async fn get_queries(
        &self,
        database_url: &str,
        region_id: Uuid,
    ) -> Result<Vec<RegionQuery>, Self::Error> {
        self.pg.get_queries(database_url, region_id).await
    }

    async fn insert_queries(
        &self,
        database_url: &str,
        region_id: Uuid,
        queries: Vec<String>,
    ) -> Result<Vec<Uuid>, Self::Error> {
        self.pg.insert_queries(database_url, region_id, queries).await
    }

    async fn create_batch(
        &self,
        database_url: &str,
        region_id: Uuid,
        expected_count: i32,
    ) -> Result<Uuid, Self::Error> {
        self.pg.create_batch(database_url, region_id, expected_count).await
    }

    async fn add_tasks_to_batch(
        &self,
        database_url: &str,
        batch_id: Uuid,
        task_ids: Vec<i64>,
    ) -> Result<(), Self::Error> {
        self.pg.add_tasks_to_batch(database_url, batch_id, task_ids).await
    }

    async fn get_batches_by_status(
        &self,
        database_url: &str,
        status: BatchStatus,
    ) -> Result<Vec<ProcessingBatch>, Self::Error> {
        self.pg.get_batches_by_status(database_url, status).await
    }

    async fn update_batch_status(
        &self,
        database_url: &str,
        batch_id: Uuid,
        status: BatchStatus,
        error: Option<String>,
    ) -> Result<(), Self::Error> {
        self.pg.update_batch_status(database_url, batch_id, status, error).await
    }

    async fn complete_batch(
        &self,
        database_url: &str,
        batch_id: Uuid,
        summary_id: Uuid,
        content_hash: String,
    ) -> Result<(), Self::Error> {
        self.pg.complete_batch(database_url, batch_id, summary_id, content_hash).await
    }

    async fn get_active_batch(
        &self,
        database_url: &str,
        region_id: Uuid,
    ) -> Result<Option<ProcessingBatch>, Self::Error> {
        self.pg.get_active_batch(database_url, region_id).await
    }
}

#[async_trait::async_trait]
impl services::RegionMappingQueries for OrchInfra {
    type Error = InfraError;

    async fn get_region_mapping(
        &self,
        database_url: &str,
        region_uuid: Uuid,
    ) -> Result<Option<services::RegionMapping>, Self::Error> {
        self.pg.get_region_mapping(database_url, region_uuid).await
    }

    async fn get_total_region_count(
        &self,
        database_url: &str,
    ) -> Result<i64, Self::Error> {
        self.pg.get_total_region_count(database_url).await
    }

    async fn count_regions_without_batches(
        &self,
        database_url: &str,
    ) -> Result<i64, Self::Error> {
        self.pg.count_regions_without_batches(database_url).await
    }
}
