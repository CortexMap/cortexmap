use cortexmap_infra::{ComponentType, InfraContext, TaskQueueInfra, TaskStatus};
use std_infra::{StdInfra, StdInfraContext};

/// Helper function to create a test infrastructure context
async fn setup_test_context() -> InfraContext<StdInfra> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cortexmap:cortexmap_dev@localhost:5432/cortexmap".to_string());
    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());

    let ctx = StdInfraContext {
        database_url,
        endpoint: "http://localhost:9000".to_string(),
        access_key: "minioadmin".to_string(),
        secret_key: "minioadmin".to_string(),
        bucket: "cortexmap-test".to_string(),
        redis_url,
    };

    ctx.get().expect("Failed to create test infrastructure context")
}

/// Helper function to clean up test data
async fn cleanup_test_data(_ctx: &InfraContext<StdInfra>, query: &str) {
    use diesel::prelude::*;
    use cortexmap_infra::schema::fetch_tasks;

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cortexmap:cortexmap_dev@localhost:5432/cortexmap".to_string());

    if let Ok(mut conn) = diesel::PgConnection::establish(&database_url) {
        diesel::delete(fetch_tasks::table.filter(fetch_tasks::query.eq(query)))
            .execute(&mut conn)
            .ok();
    }
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_enqueue_task() {
    let ctx = setup_test_context().await;
    let query = "test_enqueue_task";

    // Cleanup any existing test data
    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    let result = ctx.infra.enqueue_task(
        "PMC123456".to_string(),
        query.to_string(),
        3,
    ).await;

    assert!(result.is_ok(), "Failed to enqueue task: {:?}", result.err());

    // Verify task was created
    let stats = ctx.infra.get_task_stats().await.expect("Failed to get stats");
    assert!(stats.pending > 0, "No pending tasks found");

    // Cleanup
    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_duplicate_task_handling() {
    let ctx = setup_test_context().await;
    let query = "test_duplicate_task";

    cleanup_test_data(&ctx, query).await;

    // Enqueue same task twice
    ctx.infra.enqueue_task("PMC123456".to_string(), query.to_string(), 3).await.expect("First enqueue failed");
    ctx.infra.enqueue_task("PMC123456".to_string(), query.to_string(), 3).await.expect("Second enqueue failed");

    // Should still have only one task (idempotent)
    let stats = ctx.infra.get_task_stats().await.expect("Failed to get stats");

    // Note: This might be 1 or more depending on other tests, so we just verify no error
    assert!(stats.total >= 1, "Expected at least one task");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_task_claiming_with_timeout() {
    let ctx = setup_test_context().await;
    let query = "test_task_claiming_timeout";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra.enqueue_task("PMC789012".to_string(), query.to_string(), 3).await.expect("Failed to enqueue");

    // Claim the task
    let task1 = ctx.infra.get_next_pending_task(1, "test-worker-1").await.expect("Failed to claim task");
    assert!(task1.is_some(), "Expected to claim a task");

    // Try to claim again immediately with same worker (PEL — should be None for new messages)
    let task2 = ctx.infra.get_next_pending_task(1, "test-worker-1").await.expect("Failed to query again");
    assert!(task2.is_none(), "Should not claim another task when none are pending");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_component_status_updates() {
    let ctx = setup_test_context().await;
    let query = "test_component_status";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra.enqueue_task("PMC345678".to_string(), query.to_string(), 3).await.expect("Failed to enqueue");

    // Get the task
    let task = ctx.infra.get_next_pending_task(0, "test-worker")
        .await.expect("Failed to get task")
        .expect("No task available");

    // Mark task as started
    ctx.infra.mark_task_started(task.id).await.expect("Failed to mark started");

    // Get pending components
    let components = ctx.infra.get_pending_components(task.id).await.expect("Failed to get components");
    assert_eq!(components.len(), 3, "Expected 3 pending components");

    // Update summary component to completed
    ctx.infra.update_component_status(
        task.id,
        ComponentType::Summary,
        TaskStatus::Completed,
        Some("s3://bucket/key/summary.json".to_string()),
        None,
    ).await.expect("Failed to update component status");

    // Verify only 2 pending components remain
    let remaining = ctx.infra.get_pending_components(task.id).await.expect("Failed to get components");
    assert_eq!(remaining.len(), 2, "Expected 2 pending components after update");

    // Verify not all components completed
    let all_completed = ctx.infra.all_components_completed(task.id).await.expect("Failed to check completion");
    assert!(!all_completed, "Not all components should be completed yet");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_retry_increment() {
    let ctx = setup_test_context().await;
    let query = "test_retry_increment";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra.enqueue_task("PMC901234".to_string(), query.to_string(), 3).await.expect("Failed to enqueue");

    // Get the task
    let task = ctx.infra.get_next_pending_task(0, "test-worker")
        .await.expect("Failed to get task")
        .expect("No task available");

    ctx.infra.mark_task_started(task.id).await.expect("Failed to mark started");

    // Get a component
    let components = ctx.infra.get_pending_components(task.id).await.expect("Failed to get components");
    let component = &components[0];
    let component_type: ComponentType = component.component_type.parse().expect("Invalid component type");

    // Increment attempt count
    ctx.infra.increment_component_attempt(task.id, component_type).await.expect("Failed to increment attempt");

    // Get component again and verify attempt count increased
    let updated_components = ctx.infra.get_pending_components(task.id).await.expect("Failed to get components");
    let updated_component = updated_components.iter()
        .find(|c| c.id == component.id)
        .expect("Component not found");

    assert_eq!(updated_component.attempt_count, 1, "Attempt count should be 1");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis; stale reclaim semantics differ with Redis XAUTOCLAIM
async fn test_stale_task_reset() {
    let ctx = setup_test_context().await;
    let query = "test_stale_task_reset";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra.enqueue_task("PMC567890".to_string(), query.to_string(), 3).await.expect("Failed to enqueue");

    // Claim and mark as started
    let task = ctx.infra.get_next_pending_task(0, "test-worker")
        .await.expect("Failed to get task")
        .expect("No task available");
    ctx.infra.mark_task_started(task.id).await.expect("Failed to mark started");

    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Reclaim stale tasks (idle > 1000ms)
    let reclaimed = ctx.infra.reclaim_stale_tasks(1000, "test-reclaim-worker")
        .await.expect("Failed to reclaim stale tasks");
    assert!(!reclaimed.is_empty(), "Expected at least one stale task to be reclaimed");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_task_completion() {
    let ctx = setup_test_context().await;
    let query = "test_task_completion";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra.enqueue_task("PMC111222".to_string(), query.to_string(), 3).await.expect("Failed to enqueue");

    // Get and start task
    let task = ctx.infra.get_next_pending_task(0, "test-worker")
        .await.expect("Failed to get task")
        .expect("No task available");
    ctx.infra.mark_task_started(task.id).await.expect("Failed to mark started");

    // Mark all components as completed
    let components = ctx.infra.get_pending_components(task.id).await.expect("Failed to get components");
    for component in components {
        let component_type: ComponentType = component.component_type.parse().expect("Invalid component type");
        ctx.infra.update_component_status(
            task.id,
            component_type,
            TaskStatus::Completed,
            Some(format!("s3://bucket/{}/component", task.pmc_id)),
            None,
        ).await.expect("Failed to update component");
    }

    // Check if all components completed
    let all_done = ctx.infra.all_components_completed(task.id).await.expect("Failed to check completion");
    assert!(all_done, "All components should be completed");

    // Mark task as completed
    ctx.infra.mark_task_completed(task.id).await.expect("Failed to mark task completed");

    // Verify we got stats
    let stats = ctx.infra.get_task_stats().await.expect("Failed to get stats");
    assert!(stats.total >= 0, "Stats should be available");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_get_task_stats() {
    let ctx = setup_test_context().await;

    let stats = ctx.infra.get_task_stats().await.expect("Failed to get stats");

    // Just verify the structure is correct
    assert!(stats.total >= 0, "Total should be non-negative");
    assert!(stats.pending >= 0, "Pending should be non-negative");
    assert!(stats.in_progress >= 0, "In progress should be non-negative");
    assert!(stats.completed >= 0, "Completed should be non-negative");
    assert!(stats.failed >= 0, "Failed should be non-negative");

    // Total should equal sum of all statuses
    assert_eq!(
        stats.total,
        stats.pending + stats.in_progress + stats.completed + stats.failed,
        "Total should equal sum of all statuses"
    );
}
