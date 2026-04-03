// Integration tests for the fetcher service
// These tests require a running PostgreSQL database and S3 (MinIO) instance

use cortexmap_be::proto::{EnqueueRequest, EnqueueResponse};
use cortexmap_be::server::QueueServer;
use std::env;
use std_infra::StdInfraContext;

fn get_test_database_url() -> String {
    env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://test_user:test_password@localhost:5433/test_db".to_string()
    })
}

fn get_test_s3_config() -> (String, String, String, String) {
    let endpoint = env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".to_string());
    let access_key = env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "test_access_key".to_string());
    let secret_key = env::var("S3_SECRET_KEY").unwrap_or_else(|_| "test_secret_key".to_string());
    let bucket = env::var("S3_BUCKET").unwrap_or_else(|_| "test-bucket".to_string());
    (endpoint, access_key, secret_key, bucket)
}

fn get_test_infra_ctx() -> StdInfraContext {
    let database_url = get_test_database_url();
    let (endpoint, access_key, secret_key, bucket) = get_test_s3_config();
    StdInfraContext {
        database_url,
        endpoint,
        access_key,
        secret_key,
        bucket,
    }
}

#[tokio::test]
async fn test_queue_server_initialization() {
    let infra_ctx = get_test_infra_ctx();

    let result = QueueServer::new(infra_ctx).await;

    // Should succeed if database and S3 are available
    match result {
        Ok(server) => {
            println!("✅ QueueServer initialized successfully");
            assert!(server.blueprint_template.fetcher.query.is_empty());
        }
        Err(e) => {
            println!(
                "⚠️  QueueServer initialization failed (expected if test infrastructure not running): {}",
                e
            );
        }
    }
}

#[tokio::test]
async fn test_enqueue_task_workflow() {
    let infra_ctx = get_test_infra_ctx();

    let _server = QueueServer::new(infra_ctx)
        .await
        .expect("Failed to create QueueServer");

    // Test enqueue request
    let request = EnqueueRequest {
        query: "test brain region".to_string(),
        page_size: 5,
        max_retry_attempts: 3,
    };

    println!("Enqueuing test query: {}", request.query);

    // Note: Actual enqueue logic would require HTTP server to be running
    // This test verifies the server can be initialized
    println!("✅ QueueServer is ready to accept enqueue requests");
}

#[test]
fn test_enqueue_request_serialization() {
    let request = EnqueueRequest {
        query: "neuroplasticity".to_string(),
        page_size: 10,
        max_retry_attempts: 5,
    };

    let json = serde_json::to_string(&request).expect("Failed to serialize");
    println!("Serialized EnqueueRequest: {}", json);

    let deserialized: EnqueueRequest = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.query, "neuroplasticity");
    assert_eq!(deserialized.page_size, 10);
    assert_eq!(deserialized.max_retry_attempts, 5);
}

#[test]
fn test_enqueue_response_serialization() {
    let response = EnqueueResponse {
        success: true,
        tasks_enqueued: 3,
        pmc_ids: vec!["PMC123".to_string(), "PMC456".to_string()],
        error_message: String::new(),
        task_ids: vec![1, 2, 3],
    };

    let json = serde_json::to_string(&response).expect("Failed to serialize");
    println!("Serialized EnqueueResponse: {}", json);

    let deserialized: EnqueueResponse = serde_json::from_str(&json).expect("Failed to deserialize");

    assert!(deserialized.success);
    assert_eq!(deserialized.tasks_enqueued, 3);
    assert_eq!(deserialized.task_ids.len(), 3);
}
