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
        let api_routes = Router::new()
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
            .with_state(self);

        Router::new().nest("/fetcher-be", api_routes)
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
    use crate::proto::{TaskBreakdown, ComponentStatistics, WorkerStatistics, RecentTask};
    
    let detailed_stats = server.ctx.infra.get_detailed_task_stats().await.map_err(|e| {
        error!("Failed to get detailed task stats: {}", e);
        AppError(e.into())
    })?;
    
    let component_stats = server.ctx.infra.get_component_stats().await.map_err(|e| {
        error!("Failed to get component stats: {}", e);
        AppError(e.into())
    })?;
    
    let recent_tasks = server.ctx.infra.get_recent_tasks(10).await.map_err(|e| {
        error!("Failed to get recent tasks: {}", e);
        AppError(e.into())
    })?;
    
    let worker_manager = server.worker_manager.read().await;
    let active_workers = worker_manager.active_worker_count() as i32;
    let worker_infos = worker_manager.get_worker_info_with_stats().await;
    
    // Calculate worker statistics
    let total_tasks_processed: i64 = worker_infos.iter().map(|w| w.tasks_processed).sum();
    let avg_tasks_per_worker = if active_workers > 0 {
        total_tasks_processed as f64 / active_workers as f64
    } else {
        0.0
    };
    
    let (most_productive_worker_id, most_productive_count) = worker_infos
        .iter()
        .max_by_key(|w| w.tasks_processed)
        .map(|w| (w.worker_id.clone(), w.tasks_processed))
        .unwrap_or((String::new(), 0));
    
    // Format oldest pending task age
    let oldest_pending_age_str = detailed_stats.oldest_pending_task_age_secs
        .map(|secs| {
            let hours = secs / 3600;
            let mins = (secs % 3600) / 60;
            if hours > 0 {
                format!("{}h {}m", hours, mins)
            } else {
                format!("{}m", mins)
            }
        })
        .unwrap_or_else(|| "N/A".to_string());
    
    Ok(Json(StatusResponse {
        total_tasks: detailed_stats.basic.total,
        pending_tasks: detailed_stats.basic.pending,
        in_progress_tasks: detailed_stats.basic.in_progress,
        completed_tasks: detailed_stats.basic.completed,
        failed_tasks: detailed_stats.basic.failed,
        active_workers,
        task_breakdown: Some(TaskBreakdown {
            tasks_with_errors: detailed_stats.tasks_with_errors,
            tasks_pending_retry: detailed_stats.tasks_pending_retry,
            tasks_in_progress_over_5min: detailed_stats.tasks_in_progress_over_5min,
            average_completion_time_secs: detailed_stats.average_completion_time_secs,
            oldest_pending_task_age: oldest_pending_age_str,
        }),
        component_stats: Some(ComponentStatistics {
            total_summary_completed: component_stats.summary_completed,
            total_abstract_completed: component_stats.abstract_completed,
            total_pdf_completed: component_stats.pdf_completed,
            total_summary_failed: component_stats.summary_failed,
            total_abstract_failed: component_stats.abstract_failed,
            total_pdf_failed: component_stats.pdf_failed,
            total_components_pending: component_stats.total_pending,
        }),
        worker_stats: Some(WorkerStatistics {
            total_workers_active: active_workers as i64,
            total_workers_idle: 0, // All workers are either active or stopped
            average_tasks_per_worker: avg_tasks_per_worker,
            most_productive_worker_id,
            most_productive_worker_task_count: most_productive_count,
        }),
        recent_tasks: recent_tasks.into_iter().map(|t| RecentTask {
            pmc_id: t.pmc_id,
            status: t.status,
            created_at: t.created_at.and_utc().timestamp(),
            updated_at: t.updated_at.and_utc().timestamp(),
            worker_id: t.worker_id.unwrap_or_default(),
            components_completed: t.components_completed,
            total_components: t.total_components,
        }).collect(),
    }))
}

/// GET /api/queue/task/:pmc_id
async fn get_task_details_handler(
    State(server): State<QueueServer>,
    Path(pmc_id): Path<String>,
) -> Result<Json<TaskDetailsResponse>, AppError> {
    use crate::proto::ComponentStatus;
    
    info!("Getting task details for PMC {}", pmc_id);

    let task_opt = server.ctx.infra.get_task_by_pmc_id(&pmc_id).await.map_err(|e| {
        error!("Failed to get task by PMC ID: {}", e);
        AppError(e.into())
    })?;
    
    let task = match task_opt {
        Some(t) => t,
        None => {
            return Ok(Json(TaskDetailsResponse {
                found: false,
                pmc_id,
                status: String::new(),
                components: vec![],
                error_message: "Task not found".to_string(),
                query: String::new(),
                priority: 0,
                created_at: 0,
                updated_at: 0,
                started_at: 0,
                completed_at: 0,
                worker_id: String::new(),
                heartbeat_at: 0,
                worker_version: String::new(),
            }));
        }
    };
    
    let components_data = server.ctx.infra.get_task_components(task.id).await.map_err(|e| {
        error!("Failed to get task components: {}", e);
        AppError(e.into())
    })?;
    
    let components: Vec<ComponentStatus> = components_data.into_iter().map(|c| ComponentStatus {
        component_type: c.component_type,
        status: c.status,
        attempt_count: c.attempt_count,
        max_attempts: c.max_attempts,
        s3_key: c.s3_key.unwrap_or_default(),
        error_message: c.error_message.unwrap_or_default(),
        last_attempted_at: c.last_attempted_at.map(|dt| dt.and_utc().timestamp()).unwrap_or(0),
        completed_at: c.completed_at.map(|dt| dt.and_utc().timestamp()).unwrap_or(0),
    }).collect();
    
    Ok(Json(TaskDetailsResponse {
        found: true,
        pmc_id: task.pmc_id.clone(),
        status: task.status.clone(),
        components,
        error_message: String::new(),
        query: task.query,
        priority: task.priority,
        created_at: task.created_at.and_utc().timestamp(),
        updated_at: task.updated_at.and_utc().timestamp(),
        started_at: task.started_at.map(|dt| dt.and_utc().timestamp()).unwrap_or(0),
        completed_at: task.completed_at.map(|dt| dt.and_utc().timestamp()).unwrap_or(0),
        worker_id: task.worker_id.unwrap_or_default(),
        heartbeat_at: task.heartbeat_at.map(|dt| dt.and_utc().timestamp()).unwrap_or(0),
        worker_version: task.worker_version.unwrap_or_default(),
    }))
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
