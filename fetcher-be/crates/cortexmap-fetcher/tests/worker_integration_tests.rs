use cortexmap_core::blueprint::connections::{Connections, Database, Fetcher, Postgresql, RetryConfig, S3Info};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_fetcher::enqueue_query;
use cortexmap_infra::{ComponentType, InfraContext, TaskQueueInfra, TaskStatus};
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
            retry_config: RetryConfig::default(),
        },
        connections: Connections {
            db: Database::Postgresql(Postgresql {
                url: std::env::var("DATABASE_URL")
                    .unwrap_or_else(|_| "postgres://cortexmap:cortexmap_dev@localhost:5432/cortexmap".to_string()),
            }),
            s3_info: S3Info {
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

/// Cleanup test data
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
#[ignore] // Requires network access to NCBI API
async fn test_enqueue_query_integration() {
    let blueprint = create_test_blueprint("test_enqueue_integration AND neuroscience");
    let ctx = setup_test_context().await;

    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;

    // Enqueue tasks from a real query
    let result = enqueue_query(&blueprint, ctx).await;

    match result {
        Ok(pmc_ids) => {
            println!("✓ Enqueued {} tasks", pmc_ids.len());
            assert!(pmc_ids.len() > 0, "Expected at least one PMC ID");
            assert!(pmc_ids.len() <= blueprint.fetcher.page_size as usize, "Too many PMC IDs");

            // Verify tasks are in database
            let ctx2 = setup_test_context().await;
            let stats = ctx2.infra.get_task_stats().await.expect("Failed to get stats");
            assert!(stats.pending > 0, "No pending tasks found");
        }
        Err(e) => {
            panic!("Failed to enqueue query: {:?}", e);
        }
    }
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_process_task_lifecycle() {
    let blueprint = create_test_blueprint("test_process_lifecycle");
    let ctx = setup_test_context().await;

    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;

    // Manually enqueue a task
    ctx.infra.enqueue_task(
        "PMC5334499".to_string(), // Known good PMC ID for testing
        blueprint.fetcher.query.clone(),
        blueprint.fetcher.max_retry_attempts as i32,
    ).await.expect("Failed to enqueue task");

    // Claim the task
    let task = ctx.infra.get_next_pending_task(0, "test-worker")
        .await
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
#[ignore] // Requires running PostgreSQL and Redis
async fn test_partial_failure_retry() {
    let blueprint = create_test_blueprint("test_partial_failure");
    let ctx = setup_test_context().await;

    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;

    // Enqueue a task
    ctx.infra.enqueue_task(
        "PMC999999".to_string(), // Likely doesn't exist, will cause failures
        blueprint.fetcher.query.clone(),
        2, // Only 2 retries for faster test
    ).await.expect("Failed to enqueue task");

    // Get the task
    let task = ctx.infra.get_next_pending_task(0, "test-worker")
        .await
        .expect("Failed to get task")
        .expect("No task available");

    ctx.infra.mark_task_started(task.id).await.expect("Failed to mark started");

    // Simulate partial failure: summary succeeds, pdf fails
    let components = ctx.infra.get_pending_components(task.id).await
        .expect("Failed to get components");

    let _summary = components.iter().find(|c| c.component_type == "summary").unwrap();
    let pdf = components.iter().find(|c| c.component_type == "pdf").unwrap();

    // Mark summary as completed
    ctx.infra.update_component_status(
        task.id,
        ComponentType::Summary,
        TaskStatus::Completed,
        Some("s3://test/PMC999999/summary.json".to_string()),
        None,
    ).await.expect("Failed to update summary");

    // Increment PDF attempt count
    let _ = pdf; // suppress unused warning
    ctx.infra.increment_component_attempt(task.id, ComponentType::Pdf)
        .await.expect("Failed to increment PDF attempt");

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
#[ignore] // Requires running PostgreSQL and Redis
async fn test_timeout_prevents_immediate_retry() {
    let blueprint = create_test_blueprint("test_timeout_prevents_retry");
    let ctx = setup_test_context().await;

    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;

    // Enqueue a task
    ctx.infra.enqueue_task(
        "PMC123456".to_string(),
        blueprint.fetcher.query.clone(),
        3,
    ).await.expect("Failed to enqueue task");

    // Claim with worker-1
    let task1 = ctx.infra.get_next_pending_task(2, "worker-1")
        .await
        .expect("Failed to get task")
        .expect("No task available");

    // Same worker immediately tries again — should be None (PEL for this consumer)
    let task2 = ctx.infra.get_next_pending_task(2, "worker-1")
        .await
        .expect("Failed to query tasks");

    assert!(task2.is_none(), "Should not claim task twice with same worker");

    // task1 was claimed
    assert!(!task1.pmc_id.is_empty(), "Task should have a valid PMC ID");

    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_max_retry_exhaustion() {
    let blueprint = create_test_blueprint("test_max_retry");
    let ctx = setup_test_context().await;

    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;

    // Enqueue with only 2 max attempts
    ctx.infra.enqueue_task(
        "PMC888888".to_string(),
        blueprint.fetcher.query.clone(),
        2,
    ).await.expect("Failed to enqueue task");

    let task = ctx.infra.get_next_pending_task(0, "test-worker")
        .await
        .expect("Failed to get task")
        .expect("No task available");

    ctx.infra.mark_task_started(task.id).await.expect("Failed to mark started");

    let components = ctx.infra.get_pending_components(task.id).await
        .expect("Failed to get components");
    let pdf = components.iter().find(|c| c.component_type == "pdf").unwrap();
    let pdf_id = pdf.id;

    // Fail twice (max_attempts = 2)
    ctx.infra.increment_component_attempt(task.id, ComponentType::Pdf).await.expect("Failed");
    ctx.infra.increment_component_attempt(task.id, ComponentType::Pdf).await.expect("Failed");

    // Get component and check attempt count
    let pending = ctx.infra.get_pending_components(task.id).await
        .expect("Failed to get components");
    let pdf_updated = pending.iter().find(|c| c.id == pdf_id).unwrap();

    assert_eq!(pdf_updated.attempt_count, 2, "Should have 2 attempts");
    assert_eq!(pdf_updated.max_attempts, 2, "Max attempts should be 2");

    // In real worker, this would be marked as failed since attempt_count >= max_attempts
    if pdf_updated.attempt_count >= pdf_updated.max_attempts {
        ctx.infra.update_component_status(
            task.id,
            ComponentType::Pdf,
            TaskStatus::Failed,
            None,
            Some("Max retries exceeded".to_string()),
        ).await.expect("Failed to mark as failed");
    }

    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redis
async fn test_concurrent_workers_no_duplicate() {
    let blueprint = create_test_blueprint("test_concurrent_workers");
    let ctx = setup_test_context().await;

    cleanup_test_data(&ctx, &blueprint.fetcher.query).await;

    // Enqueue 3 tasks
    for i in 0..3 {
        ctx.infra.enqueue_task(
            format!("PMC{}", 100000 + i),
            blueprint.fetcher.query.clone(),
            3,
        ).await.expect("Failed to enqueue task");
    }

    // Simulate 3 workers claiming tasks concurrently (each with unique worker ID)
    let (task1, task2, task3) = tokio::join!(
        ctx.infra.get_next_pending_task(0, "worker-1"),
        ctx.infra.get_next_pending_task(0, "worker-2"),
        ctx.infra.get_next_pending_task(0, "worker-3"),
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
