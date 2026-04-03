use cortexmap_infra::{ComponentType, InfraContext, TaskQueueInfra};
use std::time::Instant;
use std_infra::{StdInfra, StdInfraContext};

/// Helper function to create test infrastructure context
async fn setup_test_context() -> InfraContext<StdInfra> {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://test_user:test_password@localhost:5433/test_db".to_string()
        });

    let ctx = StdInfraContext {
        database_url,
        endpoint: std::env::var("S3_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:9000".to_string()),
        access_key: std::env::var("S3_ACCESS_KEY")
            .unwrap_or_else(|_| "test_access_key".to_string()),
        secret_key: std::env::var("S3_SECRET_KEY")
            .unwrap_or_else(|_| "test_secret_key".to_string()),
        bucket: std::env::var("S3_BUCKET").unwrap_or_else(|_| "test-bucket".to_string()),
    };

    ctx.get()
        .expect("Failed to create test infrastructure context")
}

/// Cleanup test data
async fn cleanup_test_data(ctx: &InfraContext<StdInfra>, _query: &str) {
    // Cleanup is best-effort; ignore errors
    let _ = ctx.infra.reset_stale_tasks(0).await;
}

#[tokio::test]
#[ignore] // Run with: cargo test --test perf_tests -- --ignored --nocapture
async fn test_queue_throughput_batch_enqueue() {
    let ctx = setup_test_context().await;
    let query = "perf_test_batch_enqueue";
    let batch_size = 100;

    cleanup_test_data(&ctx, query).await;

    let start = Instant::now();

    // Enqueue 100 tasks
    for i in 0..batch_size {
        ctx.infra
            .enqueue_task(format!("PMC{}", 1000000 + i), query.to_string(), 3)
            .await
            .expect("Failed to enqueue task");
    }

    let elapsed = start.elapsed();
    let throughput = batch_size as f64 / elapsed.as_secs_f64();

    println!("📊 Batch Enqueue Performance:");
    println!("  Tasks: {}", batch_size);
    println!("  Time: {:.2}s", elapsed.as_secs_f64());
    println!("  Throughput: {:.2} tasks/sec", throughput);

    assert!(
        throughput > 10.0,
        "Throughput too low: {:.2} tasks/sec",
        throughput
    );

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Run with: cargo test --test perf_tests -- --ignored --nocapture
async fn test_task_claiming_latency() {
    let ctx = setup_test_context().await;
    let query = "perf_test_claiming";
    let num_tasks = 50;

    cleanup_test_data(&ctx, query).await;

    // Enqueue tasks
    for i in 0..num_tasks {
        ctx.infra
            .enqueue_task(format!("PMC{}", 2000000 + i), query.to_string(), 3)
            .await
            .expect("Failed to enqueue");
    }

    // Measure claim latency
    let mut latencies: Vec<f64> = Vec::new();

    for _ in 0..num_tasks {
        let start = Instant::now();
        let task = ctx
            .infra
            .get_next_pending_task(0)
            .await
            .expect("Failed to claim");
        let elapsed = start.elapsed();

        if task.is_some() {
            latencies.push(elapsed.as_millis() as f64);
        }
    }

    let avg_latency = latencies.iter().sum::<f64>() / latencies.len() as f64;
    let max_latency = latencies.iter().cloned().fold(0.0_f64, f64::max);
    let min_latency = latencies.iter().cloned().fold(f64::MAX, f64::min);

    println!("📊 Task Claiming Latency:");
    println!("  Samples: {}", latencies.len());
    println!("  Average: {:.2}ms", avg_latency);
    println!("  Min: {:.2}ms", min_latency);
    println!("  Max: {:.2}ms", max_latency);

    assert!(
        avg_latency < 100.0,
        "Average latency too high: {:.2}ms",
        avg_latency
    );

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Run with: cargo test --test perf_tests -- --ignored --nocapture
async fn test_concurrent_workers_throughput() {
    let ctx = setup_test_context().await;
    let query = "perf_test_concurrent";
    let num_tasks = 100;
    let num_workers = 5;

    cleanup_test_data(&ctx, query).await;

    // Enqueue tasks
    for i in 0..num_tasks {
        ctx.infra
            .enqueue_task(format!("PMC{}", 3000000 + i), query.to_string(), 3)
            .await
            .expect("Failed to enqueue");
    }

    let start = Instant::now();

    // Spawn multiple workers to claim tasks concurrently
    let mut handles = vec![];

    for worker_id in 0..num_workers {
        let ctx_clone = ctx.clone();

        let handle = tokio::spawn(async move {
            let mut claimed = 0;

            loop {
                match ctx_clone.infra.get_next_pending_task(0).await {
                    Ok(Some(task)) => {
                        claimed += 1;
                        // Mark as started to release the task
                        ctx_clone.infra.mark_task_started(task.id).await.ok();
                    }
                    Ok(None) => break, // No more tasks
                    Err(_) => break,
                }
            }

            (worker_id, claimed)
        });

        handles.push(handle);
    }

    // Wait for all workers
    let results = futures::future::join_all(handles).await;

    let elapsed = start.elapsed();
    let total_claimed: usize = results.iter().map(|r| r.as_ref().unwrap().1).sum();
    let throughput = total_claimed as f64 / elapsed.as_secs_f64();

    println!("📊 Concurrent Workers Performance:");
    println!("  Workers: {}", num_workers);
    println!("  Total tasks: {}", num_tasks);
    println!("  Tasks claimed: {}", total_claimed);
    println!("  Time: {:.2}s", elapsed.as_secs_f64());
    println!("  Throughput: {:.2} tasks/sec", throughput);

    for result in results {
        let (worker_id, claimed) = result.unwrap();
        println!("    Worker {}: {} tasks", worker_id, claimed);
    }

    // Verify all tasks were claimed (no duplicates or misses)
    assert_eq!(total_claimed, num_tasks, "Not all tasks were claimed");

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Run with: cargo test --test perf_tests -- --ignored --nocapture
async fn test_timeout_overhead() {
    let ctx = setup_test_context().await;
    let query = "perf_test_timeout";

    cleanup_test_data(&ctx, query).await;

    // Enqueue one task
    ctx.infra
        .enqueue_task("PMC4000000".to_string(), query.to_string(), 3)
        .await
        .expect("Failed to enqueue");

    // Test different timeout values
    let timeout_values = vec![0, 1, 5, 10];

    for timeout in timeout_values {
        let start = Instant::now();

        // Claim with timeout
        let _ = ctx.infra.get_next_pending_task(timeout).await;

        let elapsed = start.elapsed();

        println!(
            "⏱️  Timeout {}s: Query latency {:.2}ms",
            timeout,
            elapsed.as_millis()
        );

        // Query latency should be roughly constant regardless of timeout
        assert!(
            elapsed.as_millis() < 500,
            "Query too slow: {}ms",
            elapsed.as_millis()
        );
    }

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Run with: cargo test --test perf_tests -- --ignored --nocapture
async fn test_component_update_performance() {
    let ctx = setup_test_context().await;
    let query = "perf_test_components";
    let num_tasks = 50;

    cleanup_test_data(&ctx, query).await;

    // Enqueue tasks
    for i in 0..num_tasks {
        ctx.infra
            .enqueue_task(format!("PMC{}", 5000000 + i), query.to_string(), 3)
            .await
            .expect("Failed to enqueue");
    }

    let start = Instant::now();

    // For each task, update all 3 components
    for _ in 0..num_tasks {
        let task = ctx
            .infra
            .get_next_pending_task(0)
            .await
            .expect("Failed to claim")
            .expect("No task");

        ctx.infra
            .mark_task_started(task.id)
            .await
            .expect("Failed to mark started");

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
                    component.id,
                    component_type,
                    cortexmap_infra::TaskStatus::Completed,
                    Some(format!(
                        "s3://bucket/{}/{}",
                        task.pmc_id, component.component_type
                    )),
                    None,
                )
                .await
                .expect("Failed to update component");
        }

        ctx.infra
            .mark_task_completed(task.id)
            .await
            .expect("Failed to complete task");
    }

    let elapsed = start.elapsed();
    let throughput = num_tasks as f64 / elapsed.as_secs_f64();
    let avg_time = elapsed.as_secs_f64() / num_tasks as f64;

    println!("📊 Component Update Performance:");
    println!("  Tasks: {}", num_tasks);
    println!("  Components per task: 3");
    println!("  Total time: {:.2}s", elapsed.as_secs_f64());
    println!("  Throughput: {:.2} tasks/sec", throughput);
    println!("  Average time per task: {:.2}s", avg_time);

    cleanup_test_data(&ctx, query).await;
}

#[tokio::test]
#[ignore] // Run with: cargo test --test perf_tests -- --ignored --nocapture
async fn test_stats_query_performance() {
    let ctx = setup_test_context().await;
    let query = "perf_test_stats";

    cleanup_test_data(&ctx, query).await;

    // Enqueue varying numbers of tasks and measure stats query time
    let batch_sizes = vec![10, 50, 100, 200];

    for batch_size in batch_sizes {
        // Enqueue batch
        for i in 0..batch_size {
            ctx.infra
                .enqueue_task(format!("PMC{}", 6000000 + i), query.to_string(), 3)
                .await
                .expect("Failed to enqueue");
        }

        // Measure stats query time
        let start = Instant::now();
        let stats = ctx
            .infra
            .get_task_stats()
            .await
            .expect("Failed to get stats");
        let elapsed = start.elapsed();

        println!("📊 Stats Query Performance (total {} tasks):", stats.total);
        println!("  Query time: {:.2}ms", elapsed.as_millis());
        println!(
            "  Pending: {}, In Progress: {}, Completed: {}, Failed: {}",
            stats.pending, stats.in_progress, stats.completed, stats.failed
        );

        assert!(
            elapsed.as_millis() < 1000,
            "Stats query too slow: {}ms",
            elapsed.as_millis()
        );
    }

    cleanup_test_data(&ctx, query).await;
}
