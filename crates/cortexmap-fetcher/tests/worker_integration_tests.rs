use cortexmap_core::blueprint::connections::{Connections, Database, Fetcher, Postgresql, S3Info};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_fetcher::enqueue_query;
use cortexmap_infra::{InfraContext, TaskQueueInfra};
use std_infra::{StdInfra, StdInfraContext};

/// Helper function to create a test blueprint
fn create_test_blueprint(query: &str) -> Blueprint {
    Blueprint {
        fetcher: Fetcher {
            query: query.to_string(),
            page_size: 5,
            upload_path_prefix: "test-papers".to_string(),
            task_timeout_secs: 1,
            max_retry_attempts: 3,
            esearch_url: "https://www.ebi.ac.uk/europepmc/webservices/rest/search".to_string(),
        },
        connections: Connections {
            database: Database::Postgresql(Postgresql {
                url: std::env::var("DATABASE_URL")
                    .unwrap_or_else(|_| "postgres://cortexmap:cortexmap_dev@localhost:5432/cortexmap".to_string()),
            }),
            s3: S3Info {
                endpoint: "http://localhost:9000".to_string(),
                access_key: "minioadmin".to_string(),
                secret_key: "minioadmin".to_string(),
                bucket: "cortexmap-test".to_string(),
            },
        },
    }
}

/// Helper function to create test infrastructure context
async fn setup_test_context() -> InfraContext<StdInfra> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cortexmap:cortexmap_dev@localhost:5432/cortexmap".to_string());
    
    let ctx = StdInfraContext {
        database_url,
        endpoint: "http://localhost:9000".to_string(),
        access_key: "minioadmin".to_string(),
        secret_key: "minioadmin".to_string(),
        bucket: "cortexmap-test".to_string(),
    };
    
    ctx.get().expect("Failed to create test infrastructure context")
}

/// Cleanup test data
async fn cleanup_test_data(ctx: &InfraContext<StdInfra>, query: &str) {
    use cortexmap_infra::DatabaseInfra;
    use diesel::prelude::*;
    use cortexmap_infra::schema::fetch_tasks;
    
    let conn = ctx.infra.get_connection().expect("Failed to get connection");
    
    diesel::delete(fetch_tasks::table.filter(fetch_tasks::query.eq(query)))
        .execute(&conn)
        .ok(); // Ignore errors on cleanup
}

#[tokio::test]
#[ignore] // Requires network access to NCBI API
async fn test_enqueue_query_integration() {
    let blueprint = create_test_blueprint("test_enqueue_integration AND neuroscience");
    let ctx = setup_test_context().await;
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
    
    // Enqueue tasks from a real query
    let result = enqueue_query(&blueprint, &ctx).await;
    
    match result {
        Ok(pmc_ids) => {
            println!("✓ Enqueued {} tasks", pmc_ids.len());
            assert!(pmc_ids.len() > 0, "Expected at least one PMC ID");
            assert!(pmc_ids.len() <= blueprint.fetcher.page_size as usize, "Too many PMC IDs");
            
            // Verify tasks are in database
            let stats = ctx.infra.get_task_stats().await.expect("Failed to get stats");
            assert!(stats.pending > 0, "No pending tasks found");
        }
        Err(e) => {
            panic!("Failed to enqueue query: {:?}", e);
        }
    }
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
}

#[tokio::test]
async fn test_process_task_lifecycle() {
    let blueprint = create_test_blueprint("test_process_lifecycle");
    let ctx = setup_test_context().await;
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
    
    // Manually enqueue a task
    ctx.infra.enqueue_task(
        "PMC5334499", // Known good PMC ID for testing
        &blueprint.fetcher.query,
        blueprint.fetcher.max_retry_attempts,
    ).await.expect("Failed to enqueue task");
    
    // Claim the task
    let task = ctx.infra.get_next_pending_task(0).await
        .expect("Failed to get task")
        .expect("No task available");
    
    assert_eq!(task.pmc_id, "PMC5334499");
    assert_eq!(task.status, "pending");
    
    // Note: process_task would require network access and S3, so we just test structure
    // In a real integration test, you would call:
    // let result = process_task(task, &ctx, &blueprint).await;
    // assert!(result.is_ok());
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
}

#[tokio::test]
async fn test_partial_failure_retry() {
    let blueprint = create_test_blueprint("test_partial_failure");
    let ctx = setup_test_context().await;
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
    
    // Enqueue a task
    ctx.infra.enqueue_task(
        "PMC999999", // Likely doesn't exist, will cause failures
        &blueprint.fetcher.query,
        2, // Only 2 retries for faster test
    ).await.expect("Failed to enqueue task");
    
    // Get the task
    let task = ctx.infra.get_next_pending_task(0).await
        .expect("Failed to get task")
        .expect("No task available");
    
    ctx.infra.mark_task_started(task.id).await.expect("Failed to mark started");
    
    // Simulate partial failure: summary succeeds, pdf fails
    let components = ctx.infra.get_pending_components(task.id).await
        .expect("Failed to get components");
    
    let summary = components.iter().find(|c| c.component_type == "summary").unwrap();
    let pdf = components.iter().find(|c| c.component_type == "pdf").unwrap();
    
    // Mark summary as completed
    ctx.infra.update_component_status(
        summary.id,
        &cortexmap_infra::TaskStatus::Completed,
        Some("s3://test/PMC999999/summary.json"),
        None,
    ).await.expect("Failed to update summary");
    
    // Mark PDF as failed and increment retry
    ctx.infra.increment_component_attempt(
        pdf.id,
        Some("PDF not found"),
    ).await.expect("Failed to increment PDF attempt");
    
    // Verify summary is done but PDF is still pending
    let remaining = ctx.infra.get_pending_components(task.id).await
        .expect("Failed to get components");
    
    // Should have 2 pending (abstract and pdf), summary is completed
    assert_eq!(remaining.len(), 2, "Expected 2 pending components");
    assert!(remaining.iter().all(|c| c.component_type != "summary"), "Summary should not be pending");
    
    // Verify PDF has attempt count = 1
    let pdf_updated = remaining.iter().find(|c| c.component_type == "pdf").unwrap();
    assert_eq!(pdf_updated.attempt_count, 1, "PDF should have 1 attempt");
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
}

#[tokio::test]
async fn test_timeout_prevents_immediate_retry() {
    let blueprint = create_test_blueprint("test_timeout_prevents_retry");
    let ctx = setup_test_context().await;
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
    
    // Enqueue a task
    ctx.infra.enqueue_task(
        "PMC123456",
        &blueprint.fetcher.query,
        3,
    ).await.expect("Failed to enqueue task");
    
    // Claim with 2 second timeout
    let task1 = ctx.infra.get_next_pending_task(2).await
        .expect("Failed to get task")
        .expect("No task available");
    
    // Immediately try to claim again - should be None
    let task2 = ctx.infra.get_next_pending_task(2).await
        .expect("Failed to query tasks");
    
    assert!(task2.is_none(), "Should not claim task within timeout window");
    
    // Wait for timeout
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    
    // Now should be able to claim
    let task3 = ctx.infra.get_next_pending_task(2).await
        .expect("Failed to get task")
        .expect("Should claim task after timeout");
    
    assert_eq!(task3.id, task1.id, "Should claim same task after timeout");
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
}

#[tokio::test]
async fn test_max_retry_exhaustion() {
    let blueprint = create_test_blueprint("test_max_retry");
    let ctx = setup_test_context().await;
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
    
    // Enqueue with only 2 max attempts
    ctx.infra.enqueue_task(
        "PMC888888",
        &blueprint.fetcher.query,
        2,
    ).await.expect("Failed to enqueue task");
    
    let task = ctx.infra.get_next_pending_task(0).await
        .expect("Failed to get task")
        .expect("No task available");
    
    ctx.infra.mark_task_started(task.id).await.expect("Failed to mark started");
    
    let components = ctx.infra.get_pending_components(task.id).await
        .expect("Failed to get components");
    let pdf = components.iter().find(|c| c.component_type == "pdf").unwrap();
    
    // Fail twice (max_attempts = 2)
    ctx.infra.increment_component_attempt(pdf.id, Some("Error 1")).await.expect("Failed");
    ctx.infra.increment_component_attempt(pdf.id, Some("Error 2")).await.expect("Failed");
    
    // Get component and check attempt count
    let pending = ctx.infra.get_pending_components(task.id).await
        .expect("Failed to get components");
    let pdf_updated = pending.iter().find(|c| c.component_type == "pdf").unwrap();
    
    assert_eq!(pdf_updated.attempt_count, 2, "Should have 2 attempts");
    assert_eq!(pdf_updated.max_attempts, 2, "Max attempts should be 2");
    
    // In real worker, this would be marked as failed since attempt_count >= max_attempts
    // Let's verify the logic
    if pdf_updated.attempt_count >= pdf_updated.max_attempts {
        ctx.infra.update_component_status(
            pdf_updated.id,
            &cortexmap_infra::TaskStatus::Failed,
            None,
            Some("Max retries exceeded"),
        ).await.expect("Failed to mark as failed");
    }
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
}

#[tokio::test]
async fn test_concurrent_workers_no_duplicate() {
    let blueprint = create_test_blueprint("test_concurrent_workers");
    let ctx = setup_test_context().await;
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
    
    // Enqueue 3 tasks
    for i in 0..3 {
        ctx.infra.enqueue_task(
            &format!("PMC{}", 100000 + i),
            &blueprint.fetcher.query,
            3,
        ).await.expect("Failed to enqueue task");
    }
    
    // Simulate 3 workers claiming tasks concurrently
    let (task1, task2, task3) = tokio::join!(
        ctx.infra.get_next_pending_task(0),
        ctx.infra.get_next_pending_task(0),
        ctx.infra.get_next_pending_task(0),
    );
    
    let claimed_tasks = vec![
        task1.expect("Worker 1 failed"),
        task2.expect("Worker 2 failed"),
        task3.expect("Worker 3 failed"),
    ];
    
    // Filter out None results
    let actual_tasks: Vec<_> = claimed_tasks.into_iter().flatten().collect();
    
    // Verify no duplicates (each worker got a different task)
    let mut pmc_ids: Vec<_> = actual_tasks.iter().map(|t| &t.pmc_id).collect();
    pmc_ids.sort();
    pmc_ids.dedup();
    
    assert_eq!(pmc_ids.len(), actual_tasks.len(), "Workers claimed duplicate tasks!");
    
    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
}
