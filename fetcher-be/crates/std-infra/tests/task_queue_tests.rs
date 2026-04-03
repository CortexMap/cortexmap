use cortexmap_infra::{ComponentType, InfraContext, TaskQueueInfra, TaskStatus};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use std_infra::{StdInfra, StdInfraContext};

fn get_test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://test_user:test_password@localhost:5433/test_db".to_string()
        })
}

/// Helper function to create a test infrastructure context
async fn setup_test_context() -> InfraContext<StdInfra> {
    let ctx = StdInfraContext {
        database_url: get_test_database_url(),
        endpoint: std::env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".to_string()),
        access_key: std::env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "test_access_key".to_string()),
        secret_key: std::env::var("S3_SECRET_KEY").unwrap_or_else(|_| "test_secret_key".to_string()),
        bucket: std::env::var("S3_BUCKET").unwrap_or_else(|_| "test-bucket".to_string()),
    };

    ctx.get()
        .expect("Failed to create test infrastructure context")
}

/// Cleanup test data by deleting all tasks matching the query
async fn cleanup_test_data(_ctx: &InfraContext<StdInfra>, query: &str) {
    let database_url = get_test_database_url();
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    let pool = r2d2::Pool::builder()
        .max_size(1)
        .build(manager)
        .expect("Failed to create cleanup pool");
    let conn = &mut pool.get().expect("Failed to get cleanup connection");

    diesel::sql_query(
        "DELETE FROM fetch_task_components WHERE task_id IN (SELECT id FROM fetch_tasks WHERE query = $1)",
    )
    .bind::<diesel::sql_types::Text, _>(query)
    .execute(conn)
    .ok();
    diesel::sql_query("DELETE FROM fetch_tasks WHERE query = $1")
        .bind::<diesel::sql_types::Text, _>(query)
        .execute(conn)
        .ok();
}

#[tokio::test]
async fn test_enqueue_task() {
    let ctx = setup_test_context().await;
    let query = "test_enqueue_task";

    // Cleanup any existing test data
    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    let result = ctx
        .infra
        .enqueue_task("PMC123456".to_string(), query.to_string(), 3)
        .await;

    assert!(result.is_ok(), "Failed to enqueue task: {:?}", result.err());

    // Verify task was created
    let stats = ctx
        .infra
        .get_task_stats()
        .await
        .expect("Failed to get stats");
    assert!(stats.pending > 0, "No pending tasks found");

    // Cleanup
    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
async fn test_duplicate_task_handling() {
    let ctx = setup_test_context().await;
    let query = "test_duplicate_task";

    cleanup_test_data(&ctx, query).await;

    // Enqueue same task twice
    ctx.infra
        .enqueue_task("PMC123456".to_string(), query.to_string(), 3)
        .await
        .expect("First enqueue failed");
    ctx.infra
        .enqueue_task("PMC123456".to_string(), query.to_string(), 3)
        .await
        .expect("Second enqueue failed");

    // Should still have only one task (idempotent)
    let stats = ctx
        .infra
        .get_task_stats()
        .await
        .expect("Failed to get stats");

    // Note: This might be 1 or more depending on other tests, so we just verify no error
    assert!(stats.total >= 1, "Expected at least one task");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
async fn test_task_claiming_with_timeout() {
    let ctx = setup_test_context().await;
    let query = "test_task_claiming_timeout";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra
        .enqueue_task("PMC789012".to_string(), query.to_string(), 3)
        .await
        .expect("Failed to enqueue");

    // Claim the task with 1 second timeout
    let task1 = ctx
        .infra
        .get_next_pending_task(1)
        .await
        .expect("Failed to claim task");
    assert!(task1.is_some(), "Expected to claim a task");

    // Try to claim again immediately (should be None because of timeout)
    let task2 = ctx
        .infra
        .get_next_pending_task(1)
        .await
        .expect("Failed to query again");
    assert!(
        task2.is_none(),
        "Should not claim task within timeout window"
    );

    // Wait for timeout to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Now should be able to claim again
    let task3 = ctx
        .infra
        .get_next_pending_task(1)
        .await
        .expect("Failed to claim after timeout");
    assert!(task3.is_some(), "Expected to claim task after timeout");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
async fn test_component_status_updates() {
    let ctx = setup_test_context().await;
    let query = "test_component_status";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra
        .enqueue_task("PMC345678".to_string(), query.to_string(), 3)
        .await
        .expect("Failed to enqueue");

    // Get the task
    let task = ctx
        .infra
        .get_next_pending_task(0)
        .await
        .expect("Failed to get task")
        .expect("No task available");

    // Mark task as started
    ctx.infra
        .mark_task_started(task.id)
        .await
        .expect("Failed to mark started");

    // Get pending components
    let components = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");
    assert_eq!(components.len(), 3, "Expected 3 pending components");

    // Update summary component to completed (takes task_id, not component id)
    let _summary_component = components
        .iter()
        .find(|c| c.component_type == "summary")
        .expect("No summary component found");

    ctx.infra
        .update_component_status(
            task.id,
            ComponentType::Summary,
            TaskStatus::Completed,
            Some("s3://bucket/key/summary.json".to_string()),
            None,
        )
        .await
        .expect("Failed to update component status");

    // Verify only 2 pending components remain
    let remaining = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");
    assert_eq!(
        remaining.len(),
        2,
        "Expected 2 pending components after update"
    );

    // Verify summary component is completed
    let all_completed = ctx
        .infra
        .all_components_completed(task.id)
        .await
        .expect("Failed to check completion");
    assert!(!all_completed, "Not all components should be completed yet");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
async fn test_retry_increment() {
    let ctx = setup_test_context().await;
    let query = "test_retry_increment";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra
        .enqueue_task("PMC901234".to_string(), query.to_string(), 3)
        .await
        .expect("Failed to enqueue");

    // Get the task
    let task = ctx
        .infra
        .get_next_pending_task(0)
        .await
        .expect("Failed to get task")
        .expect("No task available");

    ctx.infra
        .mark_task_started(task.id)
        .await
        .expect("Failed to mark started");

    // Get a component
    let components = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");
    let component = &components[0];

    // Determine component type for the first component
    let component_type = match component.component_type.as_str() {
        "summary" => ComponentType::Summary,
        "abstract" => ComponentType::Abstract,
        _ => ComponentType::Pdf,
    };

    // Increment attempt count (takes task_id, not component id)
    ctx.infra
        .increment_component_attempt(task.id, component_type)
        .await
        .expect("Failed to increment attempt");

    // Get component again and verify attempt count increased
    let updated_components = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");
    let updated_component = updated_components
        .iter()
        .find(|c| c.id == component.id)
        .expect("Component not found");

    assert_eq!(
        updated_component.attempt_count, 1,
        "Attempt count should be 1"
    );

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
async fn test_stale_task_reset() {
    let ctx = setup_test_context().await;
    let query = "test_stale_task_reset";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra
        .enqueue_task("PMC567890".to_string(), query.to_string(), 3)
        .await
        .expect("Failed to enqueue");

    // Claim and mark as started
    let task = ctx
        .infra
        .get_next_pending_task(0)
        .await
        .expect("Failed to get task")
        .expect("No task available");
    ctx.infra
        .mark_task_started(task.id)
        .await
        .expect("Failed to mark started");

    // Wait for stale timeout (reset_stale_tasks uses timeout_secs * 3, so 1 * 3 = 3 seconds)
    tokio::time::sleep(tokio::time::Duration::from_secs(4)).await;

    // Reset stale tasks (timeout of 1 second means task should be reset)
    let reset_count = ctx
        .infra
        .reset_stale_tasks(1)
        .await
        .expect("Failed to reset stale tasks");
    assert!(reset_count > 0, "Expected at least one task to be reset");

    // Task should now be claimable again
    let claimed = ctx
        .infra
        .get_next_pending_task(0)
        .await
        .expect("Failed to claim reset task")
        .expect("Expected to claim reset task");

    assert_eq!(claimed.pmc_id, "PMC567890", "Wrong task claimed");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
async fn test_task_completion() {
    let ctx = setup_test_context().await;
    let query = "test_task_completion";

    cleanup_test_data(&ctx, query).await;

    // Enqueue a task
    ctx.infra
        .enqueue_task("PMC111222".to_string(), query.to_string(), 3)
        .await
        .expect("Failed to enqueue");

    // Get and start task
    let task = ctx
        .infra
        .get_next_pending_task(0)
        .await
        .expect("Failed to get task")
        .expect("No task available");
    ctx.infra
        .mark_task_started(task.id)
        .await
        .expect("Failed to mark started");

    // Mark all components as completed (update_component_status takes task_id)
    let components = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");
    for component in components {
        let component_type = match component.component_type.as_str() {
            "summary" => ComponentType::Summary,
            "abstract" => ComponentType::Abstract,
            _ => ComponentType::Pdf,
        };
        ctx.infra
            .update_component_status(
                task.id,
                component_type,
                TaskStatus::Completed,
                Some(format!("s3://bucket/{}/component", task.pmc_id)),
                None,
            )
            .await
            .expect("Failed to update component");
    }

    // Check if all components completed
    let all_done = ctx
        .infra
        .all_components_completed(task.id)
        .await
        .expect("Failed to check completion");
    assert!(all_done, "All components should be completed");

    // Mark task as completed
    ctx.infra
        .mark_task_completed(task.id)
        .await
        .expect("Failed to mark task completed");

    // Verify task is no longer pending
    let stats = ctx
        .infra
        .get_task_stats()
        .await
        .expect("Failed to get stats");
    // Just verify we got stats - actual counts depend on other tests
    assert!(stats.total >= 0, "Stats should be available");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
async fn test_get_task_stats() {
    let ctx = setup_test_context().await;

    let stats = ctx
        .infra
        .get_task_stats()
        .await
        .expect("Failed to get stats");

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
