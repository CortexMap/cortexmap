use crate::proto::*;
use crate::worker_manager::WorkerManager;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cortexmap_core::blueprint::connections::{Connections, Database, Fetcher, Postgresql, RetryConfig, S3Info};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_fetcher::enqueue_query;
use cortexmap_infra::{InfraContext, TaskQueueInfra};
use std_infra::{StdInfra, StdInfraContext};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{error, info, Level};

// ---------------------------------------------------------------------------
// Shared server state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct QueueServer {
    pub ctx: InfraContext<StdInfra>,
    pub blueprint_template: Blueprint,
    pub worker_manager: Arc<RwLock<WorkerManager>>,
}

impl QueueServer {
    pub async fn new(
        database_url: String,
        s3_endpoint: String,
        s3_access_key: String,
        s3_secret_key: String,
        s3_bucket: String,
    ) -> Result<Self, anyhow::Error> {
        let infra_ctx = StdInfraContext {
            database_url: database_url.clone(),
            endpoint: s3_endpoint.clone(),
            access_key: s3_access_key.clone(),
            secret_key: s3_secret_key.clone(),
            bucket: s3_bucket.clone(),
        };

        let ctx = infra_ctx.get()?;

        let blueprint_template = Blueprint {
            fetcher: Fetcher {
                query: String::new(),
                page_size: 10,
                upload_path_prefix: "papers".to_string(),
                task_timeout_secs: 2,
                max_retry_attempts: 3,
                esearch_url: "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pmc&term={query}&retmode=json&retmax={pageSize}".to_string(),
                retry_config: RetryConfig::default(),
            },
            connections: Connections {
                db: Database::Postgresql(Postgresql {
                    url: database_url,
                }),
                s3_info: S3Info {
                    endpoint: s3_endpoint,
                    access_key: s3_access_key,
                    secret_key: s3_secret_key,
                    bucket: s3_bucket,
                },
            },
        };

        let worker_manager = Arc::new(RwLock::new(WorkerManager::new()));

        Ok(Self {
            ctx,
            blueprint_template,
            worker_manager,
        })
    }

    /// Build the axum `Router` for all queue endpoints.
    pub fn into_router(self) -> Router {
        Router::new()
            .route("/health", get(health_handler))
            .route("/api/queue/enqueue", post(enqueue_query_handler))
            .route("/api/queue/status", get(get_queue_status_handler))
            .route("/api/queue/task/{pmc_id}", get(get_task_details_handler))
            .route("/api/queue/workers/allocate", post(allocate_workers_handler))
            .route("/api/queue/workers/stop", post(stop_workers_handler))
            .route("/api/queue/workers/status", get(get_worker_status_handler))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                    .on_response(DefaultOnResponse::new().level(Level::INFO)),
            )
            .with_state(self)
    }
}

// ---------------------------------------------------------------------------
// Error helper
// ---------------------------------------------------------------------------

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.0.to_string() });
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError(e.into())
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /health
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

/// POST /api/queue/enqueue
async fn enqueue_query_handler(
    State(server): State<QueueServer>,
    Json(req): Json<EnqueueRequest>,
) -> Result<Json<EnqueueResponse>, AppError> {
    info!("Enqueuing query: '{}' (page_size: {})", req.query, req.page_size);

    let mut blueprint = server.blueprint_template.clone();
    blueprint.fetcher.query = req.query.clone();
    blueprint.fetcher.page_size = req.page_size as u64;
    blueprint.fetcher.max_retry_attempts = req.max_retry_attempts;

    match enqueue_query(&blueprint, server.ctx.clone()).await {
        Ok(pmc_ids) => {
            info!("Successfully enqueued {} tasks", pmc_ids.len());
            Ok(Json(EnqueueResponse {
                success: true,
                tasks_enqueued: pmc_ids.len() as u32,
                pmc_ids,
                error_message: String::new(),
            }))
        }
        Err(e) => {
            error!("Failed to enqueue query: {}", e);
            Ok(Json(EnqueueResponse {
                success: false,
                tasks_enqueued: 0,
                pmc_ids: vec![],
                error_message: e.to_string(),
            }))
        }
    }
}

/// GET /api/queue/status
async fn get_queue_status_handler(
    State(server): State<QueueServer>,
) -> Result<Json<StatusResponse>, AppError> {
    let stats = server.ctx.infra.get_task_stats().await.map_err(|e| {
        error!("Failed to get queue stats: {}", e);
        AppError(e.into())
    })?;

    let worker_manager = server.worker_manager.read().await;
    let active_workers = worker_manager.active_worker_count() as i32;

    Ok(Json(StatusResponse {
        total_tasks: stats.total,
        pending_tasks: stats.pending,
        in_progress_tasks: stats.in_progress,
        completed_tasks: stats.completed,
        failed_tasks: stats.failed,
        active_workers,
    }))
}

/// GET /api/queue/task/:pmc_id
async fn get_task_details_handler(
    State(_server): State<QueueServer>,
    Path(pmc_id): Path<String>,
) -> Json<TaskDetailsResponse> {
    info!("Getting task details for PMC {}", pmc_id);

    // TODO: Implement get_task_by_pmc_id in TaskQueueInfra
    Json(TaskDetailsResponse {
        found: false,
        pmc_id,
        status: String::new(),
        components: vec![],
        error_message: "Not implemented yet".to_string(),
    })
}

/// POST /api/queue/workers/allocate
async fn allocate_workers_handler(
    State(server): State<QueueServer>,
    Json(req): Json<AllocateWorkersRequest>,
) -> Result<Json<AllocateWorkersResponse>, AppError> {
    info!("Allocating {} workers", req.worker_count);

    let mut worker_manager = server.worker_manager.write().await;

    let mut blueprint = server.blueprint_template.clone();
    blueprint.fetcher.task_timeout_secs = req.task_timeout_secs;
    blueprint.fetcher.max_retry_attempts = req.max_retry_attempts;

    match worker_manager
        .allocate_workers(req.worker_count as usize, server.ctx.clone(), blueprint)
        .await
    {
        Ok(worker_ids) => {
            info!("Successfully allocated {} workers", worker_ids.len());
            Ok(Json(AllocateWorkersResponse {
                success: true,
                worker_ids,
                error_message: String::new(),
            }))
        }
        Err(e) => {
            error!("Failed to allocate workers: {}", e);
            Ok(Json(AllocateWorkersResponse {
                success: false,
                worker_ids: vec![],
                error_message: e.to_string(),
            }))
        }
    }
}

/// POST /api/queue/workers/stop
async fn stop_workers_handler(
    State(server): State<QueueServer>,
    Json(req): Json<StopWorkersRequest>,
) -> Json<StopWorkersResponse> {
    let mut worker_manager = server.worker_manager.write().await;

    let stopped = if req.worker_ids.is_empty() {
        worker_manager.stop_all_workers().await
    } else {
        worker_manager.stop_workers(&req.worker_ids).await
    };

    Json(StopWorkersResponse {
        success: true,
        workers_stopped: stopped as u32,
        error_message: String::new(),
    })
}

/// GET /api/queue/workers/status
async fn get_worker_status_handler(
    State(server): State<QueueServer>,
) -> Json<WorkerStatusResponse> {
    let worker_manager = server.worker_manager.read().await;
    Json(WorkerStatusResponse {
        workers: worker_manager.get_worker_info_with_stats().await,
    })
}
