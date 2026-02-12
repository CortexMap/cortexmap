use crate::proto::WorkerInfo;
use cortexmap_core::blueprint::Blueprint;
use cortexmap_fetcher::worker_loop;
use cortexmap_infra::InfraContext;
use std_infra::StdInfra;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;
use uuid::Uuid;

pub struct WorkerHandle {
    pub worker_id: String,
    pub handle: JoinHandle<()>,
    pub started_at: i64,
    pub cancel_token: tokio::sync::mpsc::Sender<()>,
}

pub struct WorkerManager {
    workers: HashMap<String, WorkerHandle>,
}

impl WorkerManager {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
        }
    }

    pub async fn allocate_workers(
        &mut self,
        count: usize,
        ctx: InfraContext<StdInfra>,
        blueprint: Blueprint,
    ) -> Result<Vec<String>, anyhow::Error> {
        let mut worker_ids = Vec::new();

        for _ in 0..count {
            let worker_id = Uuid::new_v4().to_string();
            let ctx_clone = ctx.clone();
            let blueprint_clone = blueprint.clone();
            
            // Create cancellation channel
            let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel::<()>(1);

            // Spawn worker task
            let worker_id_clone = worker_id.clone();
            let worker_id_for_shutdown = worker_id.clone();
            let handle = tokio::spawn(async move {
                tokio::select! {
                    _ = cancel_rx.recv() => {
                        tracing::info!("Worker {} shutting down gracefully", worker_id_for_shutdown);
                    }
                    result = worker_loop(worker_id_clone, ctx_clone, blueprint_clone) => {
                        if let Err(e) = result {
                            tracing::error!("Worker error: {}", e);
                        }
                    }
                }
            });

            let started_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            let worker_handle = WorkerHandle {
                worker_id: worker_id.clone(),
                handle,
                started_at,
                cancel_token: cancel_tx,
            };

            self.workers.insert(worker_id.clone(), worker_handle);
            worker_ids.push(worker_id);
        }

        Ok(worker_ids)
    }

    pub async fn stop_workers(&mut self, worker_ids: &[String]) -> usize {
        let mut stopped = 0;

        for worker_id in worker_ids {
            if let Some(worker) = self.workers.remove(worker_id) {
                // Send cancellation signal
                let _ = worker.cancel_token.send(()).await;
                
                // Wait a bit for graceful shutdown
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                
                // Abort if still running
                worker.handle.abort();
                
                stopped += 1;
            }
        }

        stopped
    }

    pub async fn stop_all_workers(&mut self) -> usize {
        let worker_ids: Vec<String> = self.workers.keys().cloned().collect();
        self.stop_workers(&worker_ids).await
    }

    pub fn active_worker_count(&self) -> usize {
        self.workers.len()
    }

    pub fn get_worker_info(&self) -> Vec<WorkerInfo> {
        self.workers
            .values()
            .map(|worker| WorkerInfo {
                worker_id: worker.worker_id.clone(),
                status: if worker.handle.is_finished() {
                    "stopped".to_string()
                } else {
                    "running".to_string()
                },
                current_task: String::new(), // TODO: Track current task
                tasks_processed: 0,          // TODO: Track tasks processed
                started_at: worker.started_at,
            })
            .collect()
    }
}

impl Drop for WorkerManager {
    fn drop(&mut self) {
        // Abort all worker tasks on drop
        for (_, worker) in self.workers.drain() {
            worker.handle.abort();
        }
    }
}
