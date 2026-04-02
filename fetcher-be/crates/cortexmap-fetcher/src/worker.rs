use crate::component::{determine_component_key, fetch_component};
use crate::retry::{compute_task_backoff_delay, is_infra_retryable, with_request_retry};
use crate::FetchError;
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{
    ComponentType, ContentType, DatabaseInfra, FetchTask, HttpInfra, InfraContext, InfraError,
    NewFetchTaskLog, S3Infra, TaskQueueInfra, TaskStatus,
};
use std::time::Duration;

/// Process a single task from the queue
/// 
/// This function:
/// 1. Gets all pending components for the task
/// 2. For each component: fetch → upload to S3 → update status
/// 3. Implements retry logic based on max_attempts
/// 4. Marks task as completed when all components succeed
pub async fn process_task<I>(
    task: FetchTask,
    ctx: InfraContext<I>,
    blueprint: &Blueprint,
) -> Result<(), FetchError>
where
    I: HttpInfra + DatabaseInfra + S3Infra + TaskQueueInfra + Send + Sync + 'static,
{  
    let task_id = task.id;
    let pmc_id = task.pmc_id.clone();

    tracing::info!("Processing task {} for PMC {}", task_id, pmc_id);

    // Mark task as in progress
    ctx.infra.mark_task_started(task_id).await?;

    // Set initial Redis heartbeat TTL
    if let Some(ref sid) = task.stream_message_id {
        ctx.infra.update_task_heartbeat_redis(
            sid,
            blueprint.fetcher.retry_config.heartbeat_ttl_secs,
        ).await.ok();
    }

    // Log task start
    ctx.infra
        .log_task_event(NewFetchTaskLog {
            task_id,
            component_type: None,
            log_level: "info".to_string(),
            message: format!("Started processing task for PMC {}", pmc_id),
            metadata: None,
        })
        .await
        .ok();

    // Get all pending components
    let pending_components = ctx.infra.get_pending_components(task_id).await?;

    if pending_components.is_empty() {
        tracing::info!("No pending components for task {}, marking as completed", task_id);
        ctx.infra.mark_task_completed(task_id).await?;
        return Ok(());
    }

    tracing::info!(
        "Task {} has {} pending components",
        task_id,
        pending_components.len()
    );

    let heartbeat_interval = blueprint.fetcher.retry_config.heartbeat_interval_secs;
    let heartbeat_ttl = blueprint.fetcher.retry_config.heartbeat_ttl_secs;
    let stream_id = task.stream_message_id.clone().unwrap_or_default();
    let mut hb_interval = tokio::time::interval(Duration::from_secs(heartbeat_interval));
    hb_interval.tick().await; // consume the immediate first tick

    // Process each pending component
    for component in pending_components {
        let component_type: ComponentType = component.component_type.parse()
            .map_err(|_| FetchError::NotFound(format!("Invalid component type: {}", component.component_type)))?;

        // Use per-component retry limits from Blueprint config, falling back to global max
        let max_attempts = blueprint.fetcher.retry_config.get_component_max_retries(
            component_type.as_str(),
            blueprint.fetcher.max_retry_attempts,
        ) as i32;

        // Increment attempt counter
        let new_attempt_count = ctx
            .infra
            .increment_component_attempt(task_id, component_type)
            .await?;

        tracing::info!(
            "Processing component {:?} for task {} (attempt {}/{})",
            component_type,
            task_id,
            new_attempt_count,
            max_attempts
        );

        // Mark component as in progress
        ctx.infra
            .update_component_status(
                task_id,
                component_type,
                TaskStatus::InProgress,
                None,
                None,
            )
            .await?;

        // Try to fetch the component
        match fetch_component(pmc_id.clone(), component_type, ctx.clone()).await {
            Ok(result) => {
                // Upload to S3
                let s3_key = determine_component_key(
                    &pmc_id,
                    component_type,
                    &blueprint.fetcher.upload_path_prefix,
                );

                let content_type = match component_type {
                    ComponentType::Summary => ContentType::Json,
                    ComponentType::Abstract => ContentType::Text,
                    ComponentType::Pdf => ContentType::Pdf,
                };

                match result.into_byte_stream() {
                    Err(e) => {
                        let error_msg = format!("Failed to convert {:?} to byte stream: {}", component_type, e);
                        tracing::warn!("{}", error_msg);
                        handle_component_failure(
                            task_id,
                            component_type,
                            &error_msg,
                            new_attempt_count,
                            max_attempts,
                            &ctx,
                        )
                        .await?;
                    }
                    Ok(stream) => {
                        // Buffer the stream into bytes so we can retry the upload
                        use futures::StreamExt;
                        let mut buffer = Vec::new();
                        let mut pinned = stream;
                        while let Some(chunk) = pinned.next().await {
                            buffer.extend_from_slice(&chunk);
                        }
                        let buffered_bytes = bytes::Bytes::from(buffer);

                        // Upload to S3 with request-level retry
                        let upload_result = {
                            let infra = ctx.infra.clone();
                            let key = s3_key.clone();
                            let ct = content_type;
                            let data = buffered_bytes.clone();
                            with_request_retry(
                                || {
                                    let infra = infra.clone();
                                    let key = key.clone();
                                    let data = data.clone();
                                    async move {
                                        let stream = futures::stream::once(async move { data });
                                        infra
                                            .put_s3(&key, ct, Box::pin(stream))
                                            .await
                                            .map_err(|e| InfraError::from(e))
                                    }
                                },
                                |e| is_infra_retryable(e),
                                "S3 upload",
                            )
                                .await
                        };

                        match upload_result {
                            Ok(_) => {
                                tracing::info!(
                                    "Successfully uploaded {:?} to S3: {}",
                                    component_type,
                                    s3_key
                                );

                                // Mark component as completed
                                ctx.infra
                                    .update_component_status(
                                        task_id,
                                        component_type,
                                        TaskStatus::Completed,
                                        Some(s3_key.clone()),
                                        None,
                                    )
                                    .await?;

                                // Log success
                                ctx.infra
                                    .log_task_event(NewFetchTaskLog {
                                        task_id,
                                        component_type: Some(component_type.as_str().to_string()),
                                        log_level: "info".to_string(),
                                        message: format!("Successfully fetched and uploaded {:?}", component_type),
                                        metadata: Some(serde_json::json!({
                                            "s3_key": s3_key,
                                            "attempt": new_attempt_count
                                        })),
                                    })
                                    .await
                                    .ok();
                            }
                            Err(e) => {
                                let error_msg = format!("Failed to upload {:?} to S3: {}", component_type, e);
                                tracing::warn!("{}", error_msg);
                                handle_component_failure(
                                    task_id,
                                    component_type,
                                    &error_msg,
                                    new_attempt_count,
                                    max_attempts,
                                    &ctx,
                                )
                                    .await?;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to fetch {:?}: {}", component_type, e);
                tracing::warn!("{}", error_msg);
                handle_component_failure(
                    task_id,
                    component_type,
                    &error_msg,
                    new_attempt_count,
                    max_attempts,
                    &ctx,
                )
                .await?;
            }
        }

        // Pulse heartbeat after each component
        if !stream_id.is_empty() {
            ctx.infra.update_task_heartbeat(task_id).await.ok();
            ctx.infra.update_task_heartbeat_redis(&stream_id, heartbeat_ttl).await.ok();
        }
    }

    // Check if all components are completed
    if ctx.infra.all_components_completed(task_id).await? {
        tracing::info!("All components completed for task {}", task_id);
        ctx.infra.mark_task_completed(task_id).await?;

        // Log task completion
        ctx.infra
            .log_task_event(NewFetchTaskLog {
                task_id,
                component_type: None,
                log_level: "info".to_string(),
                message: format!("Task completed successfully for PMC {}", pmc_id),
                metadata: None,
            })
            .await
            .ok();
    } else {
        tracing::info!("Task {} has incomplete components, releasing back to pending for retry", task_id);
        
        // Release task back to pending so it can be picked up again for retry
        // This clears worker_id, heartbeat_at, started_at and sets status='pending'
        ctx.infra
            .release_task(task_id)
            .await?;
        
        // Log that task needs retry
        ctx.infra
            .log_task_event(NewFetchTaskLog {
                task_id,
                component_type: None,
                log_level: "warn".to_string(),
                message: format!("Task has incomplete components, released for retry"),
                metadata: None,
            })
            .await
            .ok();
    }

    Ok(())
}

/// Handle a component failure by either marking it for retry or as permanently failed
async fn handle_component_failure<I>(
    task_id: i64,
    component_type: ComponentType,
    error_msg: &str,
    attempt_count: i32,
    max_attempts: i32,
    ctx: &InfraContext<I>,
) -> Result<(), FetchError>
where
    I: TaskQueueInfra + Send + Sync,
{
    if attempt_count >= max_attempts {
        tracing::error!(
            "Component {:?} for task {} failed after {} attempts: {}",
            component_type,
            task_id,
            max_attempts,
            error_msg
        );

        // Mark as permanently failed
        ctx.infra
            .update_component_status(
                task_id,
                component_type,
                TaskStatus::Failed,
                None,
                Some(error_msg.to_string()),
            )
            .await?;

        // Log permanent failure
        ctx.infra
            .log_task_event(NewFetchTaskLog {
                task_id,
                component_type: Some(component_type.as_str().to_string()),
                log_level: "error".to_string(),
                message: format!(
                    "Component {:?} permanently failed after {} attempts",
                    component_type, max_attempts
                ),
                metadata: Some(serde_json::json!({
                    "error": error_msg,
                    "attempts": attempt_count
                })),
            })
            .await
            .ok();
    } else {
        tracing::info!(
            "Component {:?} for task {} will retry (attempt {}/{})",
            component_type,
            task_id,
            attempt_count,
            max_attempts
        );

        // Mark as pending for retry
        ctx.infra
            .update_component_status(
                task_id,
                component_type,
                TaskStatus::Pending,
                None,
                Some(error_msg.to_string()),
            )
            .await?;

        // Log retry
        ctx.infra
            .log_task_event(NewFetchTaskLog {
                task_id,
                component_type: Some(component_type.as_str().to_string()),
                log_level: "warn".to_string(),
                message: format!(
                    "Component {:?} failed, will retry (attempt {}/{})",
                    component_type, attempt_count, max_attempts
                ),
                metadata: Some(serde_json::json!({
                    "error": error_msg,
                    "attempt": attempt_count,
                    "max_attempts": max_attempts
                })),
            })
            .await
            .ok();
    }

    Ok(())
}

/// Main worker loop that continuously processes tasks from the queue
///
/// This function:
/// 1. Claims the next pending task (respecting timeout)
/// 2. Assigns it to this worker with heartbeat tracking
/// 3. Processes the task
/// 4. Computes backoff delay based on the configured `BackoffStrategy` and consecutive failures
/// 5. Repeats until cancelled
pub async fn worker_loop<I>(
    worker_id: String,
    ctx: InfraContext<I>,
    blueprint: Blueprint,
) -> Result<(), FetchError>
where
    I: HttpInfra + DatabaseInfra + S3Infra + TaskQueueInfra + Send + Sync + 'static,
{
    let timeout_secs = blueprint.fetcher.task_timeout_secs;
    let empty_queue_sleep_secs = blueprint.fetcher.retry_config.empty_queue_sleep_secs;
    let backoff_strategy = &blueprint.fetcher.retry_config.backoff_strategy;

    tracing::info!(
        "Starting worker {} (timeout: {}s, max retries: {}, backoff: {:?})",
        worker_id,
        timeout_secs,
        blueprint.fetcher.max_retry_attempts,
        backoff_strategy
    );

    // Track consecutive failures to escalate backoff
    let mut consecutive_failures: u32 = 0;

    loop {
        // Reclaim any stale tasks from crashed/timed-out workers
        let min_idle_ms = blueprint.fetcher.retry_config.stale_reclaim_min_idle_ms;
        match ctx.infra.reclaim_stale_tasks(min_idle_ms, &worker_id).await {
            Ok(reclaimed) if !reclaimed.is_empty() => {
                tracing::info!("Worker {} reclaimed {} stale tasks", worker_id, reclaimed.len());
                for stale_task in reclaimed {
                    if let Err(e) = process_task(stale_task.clone(), ctx.clone(), &blueprint).await {
                        tracing::error!("Worker {} error processing reclaimed task {}: {}", worker_id, stale_task.id, e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Worker {} failed to reclaim stale tasks: {}", worker_id, e);
            }
            _ => {}
        }

        // Try to claim next pending task
        match ctx.infra.get_next_pending_task(timeout_secs, &worker_id).await {
            Ok(Some(task)) => {
                tracing::info!("Worker {} claimed task {} for PMC {}", worker_id, task.id, task.pmc_id);

                // Claim the task for this worker (sets worker_id, heartbeat, status)
                if let Err(e) = ctx.infra.claim_task_for_worker(
                    task.id,
                    worker_id.clone(),
                    Some(env!("CARGO_PKG_VERSION").to_string()),
                ).await {
                    tracing::error!("Failed to claim task {} for worker {}: {}", task.id, worker_id, e);
                    consecutive_failures += 1;
                    let delay = compute_task_backoff_delay(backoff_strategy, timeout_secs, consecutive_failures);
                    tracing::debug!("Worker {} backing off for {:?} (consecutive failures: {})", worker_id, delay, consecutive_failures);
                    tokio::time::sleep(delay).await;
                    continue;
                }

                // Process the task
                let task_succeeded = match process_task(task.clone(), ctx.clone(), &blueprint).await {
                    Ok(()) => {
                        // Check if all components completed (full success) or some failed (partial)
                        match ctx.infra.all_components_completed(task.id).await {
                            Ok(true) => true,
                            _ => false,
                        }
                    }
                    Err(e) => {
                        tracing::error!("Worker {} error processing task {}: {}", worker_id, task.id, e);
                        ctx.infra
                            .mark_task_failed(task.id, format!("{}", e))
                            .await
                            .ok();
                        false
                    }
                };

                if task_succeeded {
                    // Reset consecutive failure counter on success
                    consecutive_failures = 0;
                    // Use base timeout for inter-task delay on success
                    tracing::debug!("Worker {} sleeping for {}s before next task", worker_id, timeout_secs);
                    tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
                } else {
                    // Escalate backoff on failure
                    consecutive_failures += 1;
                    let delay = compute_task_backoff_delay(backoff_strategy, timeout_secs, consecutive_failures);
                    tracing::info!(
                        "Worker {} task failed, backing off for {:?} (consecutive failures: {}, strategy: {:?})",
                        worker_id, delay, consecutive_failures, backoff_strategy
                    );
                    tokio::time::sleep(delay).await;
                }
            }
            Ok(None) => {
                // No tasks available -- reset failure counter since this isn't a failure
                consecutive_failures = 0;
                tracing::debug!("No tasks available, sleeping for {}s", empty_queue_sleep_secs);
                tokio::time::sleep(Duration::from_secs(empty_queue_sleep_secs)).await;
            }
            Err(e) => {
                tracing::error!("Error claiming task from queue: {}", e);
                consecutive_failures += 1;
                let delay = compute_task_backoff_delay(backoff_strategy, timeout_secs, consecutive_failures);
                tracing::debug!("Worker {} backing off for {:?} after queue error", worker_id, delay);
                tokio::time::sleep(delay).await;
            }
        }
    }
}

