//! Thin API facade: forwards every method to the underlying `EvalsApp`.
//! This keeps the server free of any generics over infra error types.

use crate::error::ApiError;
use app::{AppError, EvalsApp};
use rpc_types::{
    EvalSummaryResponse, ScoreRequest, ScoreResponse, ScoresForSummaryResponse, UnscoredResponse,
    WorstOffendersResponse,
};
use services::{BrainatlasClient, EnvInfra, EvalsDatabase};
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait EvalsApi: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn score(&self, req: ScoreRequest) -> Result<ScoreResponse, Self::Error>;
    async fn scores_for_summary(
        &self,
        summary_id: Uuid,
    ) -> Result<ScoresForSummaryResponse, Self::Error>;
    async fn aggregate_summary(
        &self,
        eval_version: Option<String>,
    ) -> Result<EvalSummaryResponse, Self::Error>;
    async fn worst_offenders(
        &self,
        metric: String,
        limit: i64,
        eval_version: Option<String>,
    ) -> Result<WorstOffendersResponse, Self::Error>;
    async fn brainatlas_health(&self) -> Result<(), Self::Error>;
    async fn list_unscored_summary_ids(
        &self,
        eval_version: Option<String>,
        limit: i64,
    ) -> Result<UnscoredResponse, Self::Error>;
}

pub struct Evals<DB, BC, EN, E>
where
    DB: EvalsDatabase<Error = E>,
    BC: BrainatlasClient<Error = E>,
    EN: EnvInfra<Error = E>,
    E: Error + Send + Sync + 'static,
{
    pub app: Arc<EvalsApp<DB, BC, EN, E>>,
}

impl<DB, BC, EN, E> Evals<DB, BC, EN, E>
where
    DB: EvalsDatabase<Error = E>,
    BC: BrainatlasClient<Error = E>,
    EN: EnvInfra<Error = E>,
    E: Error + Send + Sync + 'static,
{
    pub fn new(app: Arc<EvalsApp<DB, BC, EN, E>>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl<DB, BC, EN, E> EvalsApi for Evals<DB, BC, EN, E>
where
    DB: EvalsDatabase<Error = E>,
    BC: BrainatlasClient<Error = E>,
    EN: EnvInfra<Error = E>,
    E: Error + Send + Sync + 'static,
{
    type Error = ApiError<AppError<E>>;

    async fn score(&self, req: ScoreRequest) -> Result<ScoreResponse, Self::Error> {
        self.app
            .score_summary(req.summary_id, req.eval_version)
            .await
            .map_err(ApiError::AppError)
    }

    async fn scores_for_summary(
        &self,
        summary_id: Uuid,
    ) -> Result<ScoresForSummaryResponse, Self::Error> {
        self.app
            .scores_for_summary(summary_id)
            .await
            .map_err(ApiError::AppError)
    }

    async fn aggregate_summary(
        &self,
        eval_version: Option<String>,
    ) -> Result<EvalSummaryResponse, Self::Error> {
        self.app
            .aggregate_summary(eval_version)
            .await
            .map_err(ApiError::AppError)
    }

    async fn worst_offenders(
        &self,
        metric: String,
        limit: i64,
        eval_version: Option<String>,
    ) -> Result<WorstOffendersResponse, Self::Error> {
        self.app
            .worst_offenders(metric, limit, eval_version)
            .await
            .map_err(ApiError::AppError)
    }

    async fn brainatlas_health(&self) -> Result<(), Self::Error> {
        self.app.brainatlas_health().await.map_err(ApiError::AppError)
    }

    async fn list_unscored_summary_ids(
        &self,
        eval_version: Option<String>,
        limit: i64,
    ) -> Result<UnscoredResponse, Self::Error> {
        self.app
            .list_unscored_summary_ids(eval_version, limit)
            .await
            .map_err(ApiError::AppError)
    }
}
