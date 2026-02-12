use crate::component::{determine_component_key, fetch_component};
use crate::FetchError;
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{
    ComponentType, ContentType, DatabaseInfra, FetchTask, HttpInfra, InfraContext, NewFetchTaskLog,
    S3Infra, TaskQueueInfra, TaskStatus,
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

    // Process each pending component
    for component in pending_components {
        let component_type: ComponentType = component.component_type.parse()
            .map_err(|_| FetchError::NotFound(format!("Invalid component type: {}", component.component_type)))?;

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
            component.max_attempts
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
                    Ok(stream) => {
                        match ctx.infra.put_s3(&s3_key, content_type, stream).await {
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
                                    component.max_attempts,
                                    &ctx,
                                )
                                .await?;
                            }
                        }
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to convert {:?} to byte stream: {}", component_type, e);
                        tracing::warn!("{}", error_msg);
                        handle_component_failure(
                            task_id,
                            component_type,
                            &error_msg,
                            new_attempt_count,
                            component.max_attempts,
                            &ctx,
                        )
                        .await?;
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
                    component.max_attempts,
                    &ctx,
                )
                .await?;
            }
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
        tracing::info!("Task {} has incomplete components, will retry later", task_id);
        // Reset task to pending for retry
        ctx.infra
            .update_component_status(task_id, ComponentType::Summary, TaskStatus::Pending, None, None)
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
/// 4. Sleeps for the configured timeout
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

    tracing::info!(
        "Starting worker {} (timeout: {}s, max retries: {}, backoff: {:?})",
        worker_id,
        timeout_secs,
        blueprint.fetcher.max_retry_attempts,
        blueprint.fetcher.retry_config.backoff_strategy
    );

    loop {
        // Try to claim next pending task
        match ctx.infra.get_next_pending_task(timeout_secs).await {
            Ok(Some(task)) => {
                tracing::info!("Worker {} claimed task {} for PMC {}", worker_id, task.id, task.pmc_id);

                // Claim the task for this worker (sets worker_id, heartbeat, status)
                if let Err(e) = ctx.infra.claim_task_for_worker(
                    task.id,
                    worker_id.clone(),
                    Some(env!("CARGO_PKG_VERSION").to_string()),
                ).await {
                    tracing::error!("Failed to claim task {} for worker {}: {}", task.id, worker_id, e);
                    continue;
                }

                // Process the task
                if let Err(e) = process_task(task.clone(), ctx.clone(), &blueprint).await {
                    tracing::error!("Worker {} error processing task {}: {}", worker_id, task.id, e);

                    // Mark task as failed
                    ctx.infra
                        .mark_task_failed(task.id, format!("{}", e))
                        .await
                        .ok();
                }

                // Sleep for configured timeout before processing next task
                tracing::debug!("Worker {} sleeping for {}s before next task", worker_id, timeout_secs);
                tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
            }
            Ok(None) => {
                // No tasks available (either empty queue or all in timeout window)
                tracing::debug!("No tasks available, sleeping for {}s", empty_queue_sleep_secs);
                tokio::time::sleep(Duration::from_secs(empty_queue_sleep_secs)).await;
            }
            Err(e) => {
                tracing::error!("Error claiming task from queue: {}", e);
                tokio::time::sleep(Duration::from_secs(empty_queue_sleep_secs)).await;
            }
        }
    }
}

/// Reset stale tasks that have been stuck in 'in_progress' state
///
/// This is a maintenance function that should be run periodically
/// to recover from worker crashes or other failures
pub async fn reset_stale_tasks<I>(
    ctx: InfraContext<I>,
    timeout_secs: u64,
) -> Result<usize, FetchError>
where
    I: TaskQueueInfra + Send + Sync,
{
    let count = ctx.infra.reset_stale_tasks(timeout_secs).await?;
    if count > 0 {
        tracing::warn!("Reset {} stale tasks", count);
    }
    Ok(count)
}
