use cortexmap_core::blueprint::Blueprint;
use cortexmap_core::blueprint::connections::{
    Connections, Database, Fetcher, Postgresql, RetryConfig, S3Info,
};
use cortexmap_fetcher::enqueue_query;
use cortexmap_infra::{FetchTask, InfraContext, TaskQueueInfra};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use serial_test::serial;
use std_infra::{StdInfra, StdInfraContext};
use uuid::Uuid;

fn get_test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://test_user:test_password@localhost:5433/test_db".to_string()
        })
}

fn get_test_s3_config() -> (String, String, String, String) {
    let endpoint =
        std::env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".to_string());
    let access_key =
        std::env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "test_access_key".to_string());
    let secret_key =
        std::env::var("S3_SECRET_KEY").unwrap_or_else(|_| "test_secret_key".to_string());
    let bucket = std::env::var("S3_BUCKET").unwrap_or_else(|_| "test-bucket".to_string());
    (endpoint, access_key, secret_key, bucket)
}

fn unique_test_query(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

/// Helper function to create a test blueprint
fn create_test_blueprint(query: &str) -> Blueprint {
    let (endpoint, access_key, secret_key, bucket) = get_test_s3_config();
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
                url: get_test_database_url(),
            }),
            s3_info: S3Info {
                endpoint,
                access_key,
                secret_key,
                bucket,
            },
        },
    }
}

/// Helper function to create test infrastructure context
async fn setup_test_context() -> InfraContext<StdInfra> {
    let (endpoint, access_key, secret_key, bucket) = get_test_s3_config();
    let ctx = StdInfraContext {
        database_url: get_test_database_url(),
        endpoint,
        access_key,
        secret_key,
        bucket,
    };

    ctx.get()
        .expect("Failed to create test infrastructure context")
}

/// Cleanup shared queue state so tests do not interfere with one another.
async fn cleanup_test_queue() {
    let database_url = get_test_database_url();
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    let pool = r2d2::Pool::builder()
        .max_size(1)
        .build(manager)
        .expect("Failed to create cleanup pool");
    let conn = &mut pool.get().expect("Failed to get cleanup connection");

    diesel::sql_query(
        "TRUNCATE TABLE fetch_task_logs, fetch_task_components, fetch_tasks RESTART IDENTITY CASCADE",
    )
    .execute(conn)
    .expect("Failed to cleanup fetch queue tables");
}

async fn claim_task_for_test(
    ctx: InfraContext<StdInfra>,
    timeout_secs: u64,
    worker_id: &str,
) -> Result<Option<FetchTask>, cortexmap_infra::InfraError> {
    let task = ctx.infra.get_next_pending_task(timeout_secs).await?;

    if let Some(ref task) = task {
        ctx.infra
            .claim_task_for_worker(
                task.id,
                worker_id.to_string(),
                Some("test-worker".to_string()),
            )
            .await?;
    }

    Ok(task)
}

#[tokio::test]
#[serial]
#[ignore] // Requires network access to NCBI API
async fn test_enqueue_query_integration() {
    cleanup_test_queue().await;

    let blueprint = create_test_blueprint(&unique_test_query("test_enqueue_integration"));
    let ctx = setup_test_context().await;

    // Enqueue tasks from a real query
    let result = enqueue_query(&blueprint, ctx.clone()).await;

    match result {
        Ok(pmc_ids) => {
            println!("✓ Enqueued {} tasks", pmc_ids.len());
            assert!(!pmc_ids.is_empty(), "Expected at least one PMC ID");
            assert!(
                pmc_ids.len() <= blueprint.fetcher.page_size as usize,
                "Too many PMC IDs"
            );

            // Verify tasks are in database
            let stats = ctx
                .infra
                .get_task_stats()
                .await
                .expect("Failed to get stats");
            assert!(stats.pending > 0, "No pending tasks found");
        }
        Err(e) => {
            panic!("Failed to enqueue query: {:?}", e);
        }
    }

    cleanup_test_queue().await;
}

#[tokio::test]
#[serial]
async fn test_process_task_lifecycle() {
    cleanup_test_queue().await;

    let blueprint = create_test_blueprint(&unique_test_query("test_process_lifecycle"));
    let ctx = setup_test_context().await;

    // Manually enqueue a task
    let enqueued_task = ctx
        .infra
        .enqueue_task(
            "PMC5334499".to_string(), // Known good PMC ID for testing
            blueprint.fetcher.query.clone(),
            blueprint.fetcher.max_retry_attempts.try_into().unwrap(),
        )
        .await
        .expect("Failed to enqueue task");

    // Claim the task
    let task = ctx
        .infra
        .get_next_pending_task(5)
        .await
        .expect("Failed to get task")
        .expect("No task available");

    assert_eq!(task.id, enqueued_task.id);
    assert_eq!(task.pmc_id, "PMC5334499");
    assert_eq!(task.query, blueprint.fetcher.query);
    assert_eq!(task.status, "pending");

    cleanup_test_queue().await;
}

#[tokio::test]
#[serial]
async fn test_partial_failure_retry() {
    cleanup_test_queue().await;

    let blueprint = create_test_blueprint(&unique_test_query("test_partial_failure"));
    let ctx = setup_test_context().await;

    // Enqueue a task
    let enqueued_task = ctx
        .infra
        .enqueue_task(
            "PMC999999".to_string(), // Likely doesn't exist, will cause failures
            blueprint.fetcher.query.clone(),
            2, // Only 2 retries for faster test
        )
        .await
        .expect("Failed to enqueue task");

    // Get the task
    let task = ctx
        .infra
        .get_next_pending_task(5)
        .await
        .expect("Failed to get task")
        .expect("No task available");

    assert_eq!(task.id, enqueued_task.id);

    ctx.infra
        .mark_task_started(task.id)
        .await
        .expect("Failed to mark started");

    // Simulate partial failure: summary succeeds, pdf fails
    let components = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");

    let _summary = components
        .iter()
        .find(|c| c.component_type == "summary")
        .unwrap();
    let _pdf = components
        .iter()
        .find(|c| c.component_type == "pdf")
        .unwrap();

    // Mark summary as completed (update_component_status takes task_id, not component id)
    ctx.infra
        .update_component_status(
            task.id,
            cortexmap_infra::ComponentType::Summary,
            cortexmap_infra::TaskStatus::Completed,
            Some("s3://test/PMC999999/summary.json".to_string()),
            None,
        )
        .await
        .expect("Failed to update summary");

    // Mark PDF as failed and increment retry
    ctx.infra
        .increment_component_attempt(task.id, cortexmap_infra::ComponentType::Pdf)
        .await
        .expect("Failed to increment PDF attempt");

    // Verify summary is done but PDF is still pending
    let remaining = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");

    // Should have 2 pending (abstract and pdf), summary is completed
    assert_eq!(remaining.len(), 2, "Expected 2 pending components");
    assert!(
        remaining.iter().all(|c| c.component_type != "summary"),
        "Summary should not be pending"
    );

    // Verify PDF has attempt count = 1
    let pdf_updated = remaining
        .iter()
        .find(|c| c.component_type == "pdf")
        .unwrap();
    assert_eq!(pdf_updated.attempt_count, 1, "PDF should have 1 attempt");

    cleanup_test_queue().await;
}

#[tokio::test]
#[serial]
async fn test_timeout_prevents_immediate_retry() {
    cleanup_test_queue().await;

    let blueprint = create_test_blueprint(&unique_test_query("test_timeout_prevents_retry"));
    let ctx = setup_test_context().await;

    // Enqueue a task
    let enqueued_task = ctx
        .infra
        .enqueue_task("PMC123456".to_string(), blueprint.fetcher.query.clone(), 3)
        .await
        .expect("Failed to enqueue task");

    // Claim with 2 second timeout
    let task1 = ctx
        .infra
        .get_next_pending_task(2)
        .await
        .expect("Failed to get task")
        .expect("No task available");

    assert_eq!(task1.id, enqueued_task.id);

    // Immediately try to claim again - should be None
    let task2 = ctx
        .infra
        .get_next_pending_task(2)
        .await
        .expect("Failed to query tasks");

    assert!(
        task2.is_none(),
        "Should not claim task within timeout window"
    );

    // Wait for timeout
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Now should be able to claim
    let task3 = ctx
        .infra
        .get_next_pending_task(2)
        .await
        .expect("Failed to get task")
        .expect("Should claim task after timeout");

    assert_eq!(task3.id, task1.id, "Should claim same task after timeout");

    cleanup_test_queue().await;
}

#[tokio::test]
#[serial]
async fn test_max_retry_exhaustion() {
    cleanup_test_queue().await;

    let blueprint = create_test_blueprint(&unique_test_query("test_max_retry"));
    let ctx = setup_test_context().await;

    // Enqueue with only 2 max attempts
    let enqueued_task = ctx
        .infra
        .enqueue_task("PMC888888".to_string(), blueprint.fetcher.query.clone(), 2)
        .await
        .expect("Failed to enqueue task");

    let task = ctx
        .infra
        .get_next_pending_task(5)
        .await
        .expect("Failed to get task")
        .expect("No task available");

    assert_eq!(task.id, enqueued_task.id);

    ctx.infra
        .mark_task_started(task.id)
        .await
        .expect("Failed to mark started");

    let components = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");
    let _pdf = components
        .iter()
        .find(|c| c.component_type == "pdf")
        .unwrap();

    // Fail twice (max_attempts = 2) -- increment_component_attempt takes task_id
    ctx.infra
        .increment_component_attempt(task.id, cortexmap_infra::ComponentType::Pdf)
        .await
        .expect("Failed");
    ctx.infra
        .increment_component_attempt(task.id, cortexmap_infra::ComponentType::Pdf)
        .await
        .expect("Failed");

    // Get component and check attempt count
    let pending = ctx
        .infra
        .get_pending_components(task.id)
        .await
        .expect("Failed to get components");
    let pdf_updated = pending.iter().find(|c| c.component_type == "pdf").unwrap();

    assert_eq!(pdf_updated.attempt_count, 2, "Should have 2 attempts");
    assert_eq!(pdf_updated.max_attempts, 2, "Max attempts should be 2");

    // In real worker, this would be marked as failed since attempt_count >= max_attempts
    // Let's verify the logic
    if pdf_updated.attempt_count >= pdf_updated.max_attempts {
        ctx.infra
            .update_component_status(
                task.id,
                cortexmap_infra::ComponentType::Pdf,
                cortexmap_infra::TaskStatus::Failed,
                None,
                Some("Max retries exceeded".to_string()),
            )
            .await
            .expect("Failed to mark as failed");
    }

    cleanup_test_queue().await;
}

#[tokio::test]
#[serial]
async fn test_concurrent_workers_no_duplicate() {
    cleanup_test_queue().await;

    let blueprint = create_test_blueprint(&unique_test_query("test_concurrent_workers"));
    let ctx = setup_test_context().await;

    // Enqueue 3 tasks
    let mut expected_task_ids = Vec::new();
    for i in 0..3 {
        let task = ctx
            .infra
            .enqueue_task(
                format!("PMC{}", 100000 + i),
                blueprint.fetcher.query.clone(),
                3,
            )
            .await
            .expect("Failed to enqueue task");
        expected_task_ids.push(task.id);
    }

    // Simulate 3 workers claiming tasks concurrently using the same timeout semantics as the worker loop.
    let (task1, task2, task3) = tokio::join!(
        claim_task_for_test(ctx.clone(), 5, "worker-1"),
        claim_task_for_test(ctx.clone(), 5, "worker-2"),
        claim_task_for_test(ctx.clone(), 5, "worker-3"),
    );

    let claimed_tasks = vec![
        task1.expect("Worker 1 failed"),
        task2.expect("Worker 2 failed"),
        task3.expect("Worker 3 failed"),
    ];

    // Filter out None results
    let actual_tasks: Vec<_> = claimed_tasks.into_iter().flatten().collect();

    assert_eq!(
        actual_tasks.len(),
        3,
        "Expected each worker to claim one task"
    );

    // Verify no duplicates (each worker got a different task)
    let mut task_ids: Vec<_> = actual_tasks.iter().map(|t| t.id).collect();
    task_ids.sort_unstable();
    task_ids.dedup();

    expected_task_ids.sort_unstable();

    assert_eq!(
        task_ids.len(),
        actual_tasks.len(),
        "Workers claimed duplicate tasks!"
    );
    assert_eq!(
        task_ids, expected_task_ids,
        "Workers did not claim the queued tasks"
    );

    cleanup_test_queue().await;
}
