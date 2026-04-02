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
    ctx: Option<InfraContext<StdInfra>>,
}

impl WorkerManager {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
            ctx: None,
        }
    }

    pub async fn allocate_workers(
        &mut self,
        count: usize,
        ctx: InfraContext<StdInfra>,
        blueprint: Blueprint,
    ) -> Result<Vec<String>, anyhow::Error> {
        // Store ctx for later queries
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
        
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
                
                // Give worker time to finish current component heartbeat
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                
                // Release any tasks this worker holds back to the queue
                // (reclaim_stale_tasks on next poll cycle will pick them up)
                // For now abort cleanly — stale reclaim handles orphaned PEL entries
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
        use std::time::{SystemTime, UNIX_EPOCH};
        
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        
        self.workers
            .values()
            .map(|worker| {
                let uptime_secs = (current_time - worker.started_at) as f64;
                
                WorkerInfo {
                    worker_id: worker.worker_id.clone(),
                    status: if worker.handle.is_finished() {
                        "stopped".to_string()
                    } else {
                        "running".to_string()
                    },
                    current_task: String::new(), // Will be populated by async query
                    tasks_processed: 0,          // Will be populated by async query
                    started_at: worker.started_at,
                    worker_version: String::new(),
                    last_heartbeat_at: 0,
                    uptime_seconds: uptime_secs,
                    tasks_failed: 0,
                    success_rate: 0.0,
                }
            })
            .collect()
    }
    
    /// Get detailed worker info with task counts from database
    pub async fn get_worker_info_with_stats(&self) -> Vec<WorkerInfo> {
        let mut infos = self.get_worker_info();
        
        if let Some(ref ctx) = self.ctx {
            // Query database for each worker's stats in parallel
            let worker_ids: Vec<String> = infos.iter().map(|i| i.worker_id.clone()).collect();
            
            for (info, worker_id) in infos.iter_mut().zip(worker_ids.iter()) {
                // Get stats from database using a raw SQL query via task queue infrastructure
                if let Ok(stats) = self.query_worker_stats(worker_id, ctx).await {
                    info.tasks_processed = stats.completed_count;
                    info.current_task = stats.current_pmc;
                    info.tasks_failed = stats.failed_count;
                    info.worker_version = stats.worker_version;
                    info.last_heartbeat_at = stats.last_heartbeat_ts;
                    
                    // Calculate success rate
                    let total_processed = stats.completed_count + stats.failed_count;
                    if total_processed > 0 {
                        info.success_rate = stats.completed_count as f64 / total_processed as f64;
                    }
                }
            }
        }
        
        infos
    }
    
    async fn query_worker_stats(&self, worker_id: &str, ctx: &InfraContext<StdInfra>) -> Result<WorkerStatsResult, anyhow::Error> {
        use diesel::prelude::*;
        use diesel::sql_types::{Text, BigInt, Nullable};
        
        #[derive(QueryableByName)]
        struct WorkerStats {
            #[diesel(sql_type = BigInt)]
            completed_count: i64,
            #[diesel(sql_type = BigInt)]
            failed_count: i64,
            #[diesel(sql_type = Text)]
            current_pmc: String,
            #[diesel(sql_type = Nullable<Text>)]
            worker_version: Option<String>,
            #[diesel(sql_type = Nullable<BigInt>)]
            last_heartbeat_ts: Option<i64>,
        }
        
        let worker_id = worker_id.to_string();
        let pool = ctx.infra.db_pool().clone();
        
        let stats = tokio::task::spawn_blocking(move || -> Result<WorkerStats, anyhow::Error> {
            let mut conn = pool.get()?;
            
            let query = diesel::sql_query(
                "SELECT 
                    COUNT(*) FILTER (WHERE status = 'completed') as completed_count,
                    COUNT(*) FILTER (WHERE status = 'failed') as failed_count,
                    COALESCE(MAX(pmc_id) FILTER (WHERE status = 'in_progress'), '') as current_pmc,
                    MAX(worker_version) as worker_version,
                    MAX(EXTRACT(EPOCH FROM heartbeat_at))::BIGINT as last_heartbeat_ts
                 FROM fetch_tasks 
                 WHERE worker_id = $1"
            )
            .bind::<Text, _>(&worker_id);
            
            let result = query.get_result::<WorkerStats>(&mut conn)?;
            Ok(result)
        })
        .await??;
        
        Ok(WorkerStatsResult {
            completed_count: stats.completed_count,
            failed_count: stats.failed_count,
            current_pmc: stats.current_pmc,
            worker_version: stats.worker_version.unwrap_or_default(),
            last_heartbeat_ts: stats.last_heartbeat_ts.unwrap_or(0),
        })
    }
}

struct WorkerStatsResult {
    completed_count: i64,
    failed_count: i64,
    current_pmc: String,
    worker_version: String,
    last_heartbeat_ts: i64,
}

impl Drop for WorkerManager {
    fn drop(&mut self) {
        // Abort all worker tasks on drop
        for (_, worker) in self.workers.drain() {
            worker.handle.abort();
        }
    }
}
