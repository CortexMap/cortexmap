use crate::proto::queue_service_server::{QueueService as QueueServiceTrait, QueueServiceServer};
use crate::proto::*;
use crate::worker_manager::WorkerManager;
use cortexmap_core::blueprint::connections::{Connections, Database, Fetcher, Postgresql, S3Info};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_fetcher::enqueue_query;
use cortexmap_infra::{InfraContext, TaskQueueInfra};
use std_infra::{StdInfra, StdInfraContext};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{error, info};

pub struct QueueServer {
    ctx: InfraContext<StdInfra>,
    blueprint_template: Blueprint,
    worker_manager: Arc<RwLock<WorkerManager>>,
}

impl QueueServer {
    pub async fn new(
        database_url: String,
        s3_endpoint: String,
        s3_access_key: String,
        s3_secret_key: String,
        s3_bucket: String,
    ) -> Result<Self, anyhow::Error> {
        // Create infrastructure context
        let infra_ctx = StdInfraContext {
            database_url: database_url.clone(),
            endpoint: s3_endpoint.clone(),
            access_key: s3_access_key.clone(),
            secret_key: s3_secret_key.clone(),
            bucket: s3_bucket.clone(),
        };

        let ctx = infra_ctx.get()?;

        // Create blueprint template
        let blueprint_template = Blueprint {
            fetcher: Fetcher {
                query: String::new(), // Will be filled per request
                page_size: 10,
                upload_path_prefix: "papers".to_string(),
                task_timeout_secs: 2,
                max_retry_attempts: 3,
                esearch_url: "https://www.ebi.ac.uk/europepmc/webservices/rest/search".to_string(),
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

        // Create worker manager
        let worker_manager = Arc::new(RwLock::new(WorkerManager::new()));

        Ok(Self {
            ctx,
            blueprint_template,
            worker_manager,
        })
    }

    pub fn into_service(self) -> QueueServiceServer<Self> {
        QueueServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl QueueServiceTrait for QueueServer {
    async fn enqueue_query(
        &self,
        request: Request<EnqueueRequest>,
    ) -> Result<Response<EnqueueResponse>, Status> {
        let req = request.into_inner();
        info!("Enqueuing query: '{}' (page_size: {})", req.query, req.page_size);

        // Create blueprint for this query
        let mut blueprint = self.blueprint_template.clone();
        blueprint.fetcher.query = req.query.clone();
        blueprint.fetcher.page_size = req.page_size as u64;
        blueprint.fetcher.max_retry_attempts = req.max_retry_attempts;

        // Enqueue tasks
        match enqueue_query(&blueprint, self.ctx.clone()).await {
            Ok(pmc_ids) => {
                info!("Successfully enqueued {} tasks", pmc_ids.len());
                Ok(Response::new(EnqueueResponse {
                    success: true,
                    tasks_enqueued: pmc_ids.len() as u32,
                    pmc_ids,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!("Failed to enqueue query: {}", e);
                Ok(Response::new(EnqueueResponse {
                    success: false,
                    tasks_enqueued: 0,
                    pmc_ids: vec![],
                    error_message: e.to_string(),
                }))
            }
        }
    }

    async fn get_queue_status(
        &self,
        _request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        match self.ctx.infra.get_task_stats().await {
            Ok(stats) => {
                let worker_manager = self.worker_manager.read().await;
                let active_workers = worker_manager.active_worker_count() as i32;

                Ok(Response::new(StatusResponse {
                    total_tasks: stats.total,
                    pending_tasks: stats.pending,
                    in_progress_tasks: stats.in_progress,
                    completed_tasks: stats.completed,
                    failed_tasks: stats.failed,
                    active_workers,
                }))
            }
            Err(e) => {
                error!("Failed to get queue stats: {}", e);
                Err(Status::internal(format!("Failed to get stats: {}", e)))
            }
        }
    }

    async fn get_task_details(
        &self,
        request: Request<TaskDetailsRequest>,
    ) -> Result<Response<TaskDetailsResponse>, Status> {
        let req = request.into_inner();
        
        // This would require a new method in TaskQueueInfra to get task by PMC ID
        // For now, return a placeholder
        info!("Getting task details for PMC {}", req.pmc_id);
        
        // TODO: Implement get_task_by_pmc_id in TaskQueueInfra
        Ok(Response::new(TaskDetailsResponse {
            found: false,
            pmc_id: req.pmc_id,
            status: String::new(),
            components: vec![],
            error_message: "Not implemented yet".to_string(),
        }))
    }

    async fn allocate_workers(
        &self,
        request: Request<AllocateWorkersRequest>,
    ) -> Result<Response<AllocateWorkersResponse>, Status> {
        let req = request.into_inner();
        info!("Allocating {} workers", req.worker_count);

        let mut worker_manager = self.worker_manager.write().await;
        
        // Create blueprint for workers
        let mut blueprint = self.blueprint_template.clone();
        blueprint.fetcher.task_timeout_secs = req.task_timeout_secs;
        blueprint.fetcher.max_retry_attempts = req.max_retry_attempts;

        match worker_manager.allocate_workers(
            req.worker_count as usize,
            self.ctx.clone(),
            blueprint,
        ).await {
            Ok(worker_ids) => {
                info!("Successfully allocated {} workers", worker_ids.len());
                Ok(Response::new(AllocateWorkersResponse {
                    success: true,
                    worker_ids,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!("Failed to allocate workers: {}", e);
                Ok(Response::new(AllocateWorkersResponse {
                    success: false,
                    worker_ids: vec![],
                    error_message: e.to_string(),
                }))
            }
        }
    }

    async fn stop_workers(
        &self,
        request: Request<StopWorkersRequest>,
    ) -> Result<Response<StopWorkersResponse>, Status> {
        let req = request.into_inner();
        let mut worker_manager = self.worker_manager.write().await;

        let stopped = if req.worker_ids.is_empty() {
            // Stop all workers
            worker_manager.stop_all_workers().await
        } else {
            // Stop specific workers
            worker_manager.stop_workers(&req.worker_ids).await
        };

        Ok(Response::new(StopWorkersResponse {
            success: true,
            workers_stopped: stopped as u32,
            error_message: String::new(),
        }))
    }

    async fn get_worker_status(
        &self,
        _request: Request<WorkerStatusRequest>,
    ) -> Result<Response<WorkerStatusResponse>, Status> {
        let worker_manager = self.worker_manager.read().await;
        let workers = worker_manager.get_worker_info();

        Ok(Response::new(WorkerStatusResponse { workers }))
    }

    type StreamQueueStatusStream = ReceiverStream<Result<StatusResponse, Status>>;

    async fn stream_queue_status(
        &self,
        request: Request<StreamStatusRequest>,
    ) -> Result<Response<Self::StreamQueueStatusStream>, Status> {
        let req = request.into_inner();
        let interval_seconds = if req.interval_seconds == 0 {
            5
        } else {
            req.interval_seconds
        };

        let (tx, rx) = tokio::sync::mpsc::channel(128);
        let ctx = self.ctx.clone();
        let worker_manager = self.worker_manager.clone();

        // Spawn background task to send periodic updates
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_seconds as u64));
            
            loop {
                interval.tick().await;
                
                match ctx.infra.get_task_stats().await {
                    Ok(stats) => {
                        let worker_manager = worker_manager.read().await;
                        let active_workers = worker_manager.active_worker_count() as i32;

                        let response = StatusResponse {
                            total_tasks: stats.total,
                            pending_tasks: stats.pending,
                            in_progress_tasks: stats.in_progress,
                            completed_tasks: stats.completed,
                            failed_tasks: stats.failed,
                            active_workers,
                        };

                        if tx.send(Ok(response)).await.is_err() {
                            // Client disconnected
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to get stats for stream: {}", e);
                        if tx.send(Err(Status::internal(e.to_string()))).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
