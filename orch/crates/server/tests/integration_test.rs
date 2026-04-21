// Integration tests for Orch (Orchestration Service)
// These tests require PostgreSQL, MinIO, Fetcher, and BrainAtlas services
//
// To run: cargo test --package server --test integration_test -- --test-threads=1
//
// Prerequisites:
// 1. docker-compose -f ../../../docker-compose.test.yml up -d
// 2. diesel migration run --database-url postgresql://test_user:test_password@localhost:5433/test_db

use std::env;
use uuid::Uuid;

// QueryableByName types for diesel
#[derive(diesel::QueryableByName)]
struct CountResult {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

#[derive(diesel::QueryableByName)]
struct SelectOne {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    value: i32,
}

#[derive(diesel::QueryableByName)]
struct RegionMappingRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    region_id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
}

#[derive(diesel::QueryableByName)]
struct BatchRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    status: String,
}

#[derive(diesel::QueryableByName)]
struct ConfigRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    key: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    value: String,
}

// Helper to get test database URL
fn get_test_db_url() -> String {
    env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://test_user:test_password@localhost:5433/test_db".to_string()
    })
}

// Helper to get database connection
fn get_db_connection() -> diesel::PgConnection {
    use diesel::prelude::*;
    let url = get_test_db_url();
    PgConnection::establish(&url).expect("Failed to connect to test database")
}

mod database_tests {
    use super::*;
    use diesel::prelude::*;

    #[tokio::test]

    async fn test_database_connection() {
        let mut conn = get_db_connection();

        // Simple query to verify connection
        let result = diesel::sql_query("SELECT 1 as value")
            .get_result::<SelectOne>(&mut conn)
            .expect("Failed to execute query");

        assert_eq!(result.value, 1, "Database connection works");
        println!("✅ Database connection successful");
    }

    #[tokio::test]

    async fn test_region_mapping_table_exists() {
        let mut conn = get_db_connection();

        // Check if region_mapping table exists
        let count = diesel::sql_query(
            "SELECT COUNT(*) as count FROM information_schema.tables 
             WHERE table_name = 'region_mapping'",
        )
        .get_result::<CountResult>(&mut conn)
        .expect("Failed to check table existence");

        assert_eq!(count.count, 1, "region_mapping table should exist");
        println!("✅ region_mapping table exists");
    }

    #[tokio::test]

    async fn test_region_processing_batches_table_exists() {
        let mut conn = get_db_connection();

        // Check if region_processing_batches table exists
        let count = diesel::sql_query(
            "SELECT COUNT(*) as count FROM information_schema.tables 
             WHERE table_name = 'region_processing_batches'",
        )
        .get_result::<CountResult>(&mut conn)
        .expect("Failed to check table existence");

        assert_eq!(
            count.count, 1,
            "region_processing_batches table should exist"
        );
        println!("✅ region_processing_batches table exists");
    }

    #[tokio::test]

    async fn test_config_table_exists() {
        let mut conn = get_db_connection();

        // Check if orch_config table exists
        let count = diesel::sql_query(
            "SELECT COUNT(*) as count FROM information_schema.tables 
             WHERE table_name = 'orch_config'",
        )
        .get_result::<CountResult>(&mut conn)
        .expect("Failed to check table existence");

        assert_eq!(count.count, 1, "orch_config table should exist");
        println!("✅ orch_config table exists");
    }

    #[tokio::test]

    async fn test_insert_and_query_region_mapping() {
        let mut conn = get_db_connection();

        let test_region_id = 99999;
        let test_uuid = Uuid::new_v4();

        // Insert test region
        diesel::sql_query(
            "INSERT INTO region_mapping (id, region_id, name, acronym) 
             VALUES ($1, $2, $3, $4)",
        )
        .bind::<diesel::sql_types::Uuid, _>(test_uuid)
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>("Test Region for Orch")
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("TRO"))
        .execute(&mut conn)
        .expect("Failed to insert test region");

        // Query it back
        let region = diesel::sql_query(
            "SELECT id, region_id, name FROM region_mapping WHERE region_id = $1",
        )
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .get_result::<RegionMappingRow>(&mut conn)
        .expect("Failed to query region");

        assert_eq!(region.id, test_uuid);
        assert_eq!(region.region_id, test_region_id);
        assert_eq!(region.name, "Test Region for Orch");

        // Cleanup
        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(test_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ Region mapping CRUD operations work");
    }
}

mod config_tests {
    use super::*;
    use diesel::prelude::*;

    #[tokio::test]

    async fn test_insert_and_retrieve_config() {
        let mut conn = get_db_connection();

        let test_key = format!("test_key_{}", Uuid::new_v4());
        let test_value = "test_value_123";

        // Insert config
        diesel::sql_query("INSERT INTO orch_config (key, value, description) VALUES ($1, $2, $3)")
            .bind::<diesel::sql_types::Text, _>(&test_key)
            .bind::<diesel::sql_types::Text, _>(test_value)
            .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("Test config"))
            .execute(&mut conn)
            .expect("Failed to insert config");

        // Retrieve it
        let config = diesel::sql_query("SELECT key, value FROM orch_config WHERE key = $1")
            .bind::<diesel::sql_types::Text, _>(&test_key)
            .get_result::<ConfigRow>(&mut conn)
            .expect("Failed to retrieve config");

        assert_eq!(config.key, test_key);
        assert_eq!(config.value, test_value);

        // Update it
        diesel::sql_query("UPDATE orch_config SET value = $1 WHERE key = $2")
            .bind::<diesel::sql_types::Text, _>("updated_value")
            .bind::<diesel::sql_types::Text, _>(&test_key)
            .execute(&mut conn)
            .expect("Failed to update config");

        // Verify update
        let updated_config = diesel::sql_query("SELECT key, value FROM orch_config WHERE key = $1")
            .bind::<diesel::sql_types::Text, _>(&test_key)
            .get_result::<ConfigRow>(&mut conn)
            .expect("Failed to retrieve updated config");

        assert_eq!(updated_config.value, "updated_value");

        // Cleanup
        diesel::sql_query("DELETE FROM orch_config WHERE key = $1")
            .bind::<diesel::sql_types::Text, _>(&test_key)
            .execute(&mut conn)
            .ok();

        println!("✅ Config CRUD operations work");
    }
}

mod batch_tests {
    use super::*;
    use diesel::prelude::*;

    #[tokio::test]

    async fn test_create_and_query_batch() {
        let mut conn = get_db_connection();

        // First create a test region
        let test_region_uuid = Uuid::new_v4();
        let test_region_id = (rand::random::<u16>() as i32) + 10000; // Random ID 10000-75535

        diesel::sql_query("INSERT INTO region_mapping (id, region_id, name) VALUES ($1, $2, $3)")
            .bind::<diesel::sql_types::Uuid, _>(test_region_uuid)
            .bind::<diesel::sql_types::Int4, _>(test_region_id)
            .bind::<diesel::sql_types::Text, _>(&format!("Test Batch Region {}", test_region_uuid))
            .execute(&mut conn)
            .expect("Failed to insert test region");

        // Create a batch (region_id is UUID FK to region_mapping.id)
        let batch_id = Uuid::new_v4();
        diesel::sql_query(
            "INSERT INTO region_processing_batches 
             (id, region_id, status, expected_task_count, created_at) 
             VALUES ($1, $2, $3, $4, NOW())",
        )
        .bind::<diesel::sql_types::Uuid, _>(batch_id)
        .bind::<diesel::sql_types::Uuid, _>(test_region_uuid)
        .bind::<diesel::sql_types::Text, _>("collecting")
        .bind::<diesel::sql_types::Int4, _>(5)
        .execute(&mut conn)
        .expect("Failed to insert batch");

        // Query it back
        let batch =
            diesel::sql_query("SELECT id, status FROM region_processing_batches WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(batch_id)
                .get_result::<BatchRow>(&mut conn)
                .expect("Failed to query batch");

        assert_eq!(batch.id, batch_id);
        assert_eq!(batch.status, "collecting");

        // Update status
        diesel::sql_query("UPDATE region_processing_batches SET status = $1 WHERE id = $2")
            .bind::<diesel::sql_types::Text, _>("ready")
            .bind::<diesel::sql_types::Uuid, _>(batch_id)
            .execute(&mut conn)
            .expect("Failed to update batch status");

        // Verify update
        let updated_batch =
            diesel::sql_query("SELECT id, status FROM region_processing_batches WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(batch_id)
                .get_result::<BatchRow>(&mut conn)
                .expect("Failed to query updated batch");

        assert_eq!(updated_batch.status, "ready");

        // Cleanup
        diesel::sql_query("DELETE FROM region_processing_batches WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(batch_id)
            .execute(&mut conn)
            .ok();

        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(test_region_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ Batch CRUD operations work");
    }

    #[tokio::test]

    async fn test_count_batches_by_status() {
        let mut conn = get_db_connection();

        // Create batches with different statuses (each needs own region due to unique constraint)
        let statuses = ["collecting", "ready", "completed"];

        let mut batch_ids = Vec::new();
        let mut region_uuids = Vec::new();

        for status in &statuses {
            let region_uuid = Uuid::new_v4();
            let region_id = (rand::random::<u16>() as i32) + 10000;

            // Create region
            diesel::sql_query(
                "INSERT INTO region_mapping (id, region_id, name) VALUES ($1, $2, $3)",
            )
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .bind::<diesel::sql_types::Int4, _>(region_id)
            .bind::<diesel::sql_types::Text, _>(&format!("Test {} Region {}", status, region_uuid))
            .execute(&mut conn)
            .expect("Failed to insert test region");

            // Create batch (region_id is UUID FK to region_mapping.id)
            let batch_id = Uuid::new_v4();
            diesel::sql_query(
                "INSERT INTO region_processing_batches 
                 (id, region_id, status, expected_task_count, created_at) 
                 VALUES ($1, $2, $3, $4, NOW())",
            )
            .bind::<diesel::sql_types::Uuid, _>(batch_id)
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .bind::<diesel::sql_types::Text, _>(status)
            .bind::<diesel::sql_types::Int4, _>(1)
            .execute(&mut conn)
            .expect("Failed to insert batch");

            batch_ids.push(batch_id);
            region_uuids.push(region_uuid);
        }

        // Count by status
        let collecting_count = diesel::sql_query(
            "SELECT COUNT(*) as count FROM region_processing_batches 
             WHERE status = 'collecting'",
        )
        .get_result::<CountResult>(&mut conn)
        .expect("Failed to count collecting batches");

        assert!(
            collecting_count.count >= 1,
            "Should have at least 1 collecting batch"
        );

        // Cleanup
        for batch_id in batch_ids {
            diesel::sql_query("DELETE FROM region_processing_batches WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(batch_id)
                .execute(&mut conn)
                .ok();
        }

        for region_uuid in region_uuids {
            diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(region_uuid)
                .execute(&mut conn)
                .ok();
        }

        println!("✅ Batch counting by status works");
    }
}

mod workflow_tests {
    use super::*;
    use diesel::prelude::*;

    #[tokio::test]

    async fn test_region_search_workflow_simulation() {
        let mut conn = get_db_connection();

        // 1. Create a test region
        let region_uuid = Uuid::new_v4();
        let region_id = (rand::random::<u16>() as i32) + 10000;

        diesel::sql_query(
            "INSERT INTO region_mapping (id, region_id, name, acronym) 
             VALUES ($1, $2, $3, $4)",
        )
        .bind::<diesel::sql_types::Uuid, _>(region_uuid)
        .bind::<diesel::sql_types::Int4, _>(region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Workflow Test Region {}", region_uuid))
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("WTR"))
        .execute(&mut conn)
        .expect("Failed to insert test region");

        // 2. Simulate search request - check if summaries exist (should be empty)
        let summary_count =
            diesel::sql_query("SELECT COUNT(*) as count FROM region_summary WHERE region_id = $1")
                .bind::<diesel::sql_types::Int4, _>(region_id)
                .get_result::<CountResult>(&mut conn)
                .expect("Failed to count summaries");

        assert_eq!(
            summary_count.count, 0,
            "New region should have no summaries"
        );

        // 3. Check if batch exists (should not exist) -- region_id is UUID FK
        let batch_count = diesel::sql_query(
            "SELECT COUNT(*) as count FROM region_processing_batches WHERE region_id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(region_uuid)
        .get_result::<CountResult>(&mut conn)
        .expect("Failed to count batches");

        assert_eq!(batch_count.count, 0, "New region should have no batches");

        // 4. Create a new batch (simulating enqueue) -- region_id is UUID FK
        let batch_id = Uuid::new_v4();
        diesel::sql_query(
            "INSERT INTO region_processing_batches 
             (id, region_id, status, expected_task_count, created_at) 
             VALUES ($1, $2, 'collecting', 3, NOW())",
        )
        .bind::<diesel::sql_types::Uuid, _>(batch_id)
        .bind::<diesel::sql_types::Uuid, _>(region_uuid)
        .execute(&mut conn)
        .expect("Failed to create batch");

        // 5. Simulate task completion - update to ready
        diesel::sql_query("UPDATE region_processing_batches SET status = 'ready' WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(batch_id)
            .execute(&mut conn)
            .expect("Failed to update batch to ready");

        // 6. Simulate processing
        diesel::sql_query(
            "UPDATE region_processing_batches 
             SET status = 'processing', processing_started_at = NOW() 
             WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(batch_id)
        .execute(&mut conn)
        .expect("Failed to update batch to processing");

        // 7. Simulate completion - create summary (batch_id is required)
        let summary_id = Uuid::new_v4();
        diesel::sql_query(
            "INSERT INTO region_summary (id, region_id, name, summary, batch_id, created_at) 
             VALUES ($1, $2, $3, $4, $5, NOW())",
        )
        .bind::<diesel::sql_types::Uuid, _>(summary_id)
        .bind::<diesel::sql_types::Int4, _>(region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Workflow Test Region {}", region_uuid))
        .bind::<diesel::sql_types::Text, _>("This is a test summary from workflow simulation")
        .bind::<diesel::sql_types::Uuid, _>(batch_id)
        .execute(&mut conn)
        .expect("Failed to create summary");

        // 8. Mark batch as completed
        diesel::sql_query(
            "UPDATE region_processing_batches 
             SET status = 'completed', completed_at = NOW() 
             WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(batch_id)
        .execute(&mut conn)
        .expect("Failed to update batch to completed");

        // 9. Verify final state
        let final_batch =
            diesel::sql_query("SELECT id, status FROM region_processing_batches WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(batch_id)
                .get_result::<BatchRow>(&mut conn)
                .expect("Failed to query final batch state");

        assert_eq!(final_batch.status, "completed");

        let final_summary_count =
            diesel::sql_query("SELECT COUNT(*) as count FROM region_summary WHERE region_id = $1")
                .bind::<diesel::sql_types::Int4, _>(region_id)
                .get_result::<CountResult>(&mut conn)
                .expect("Failed to count final summaries");

        assert_eq!(
            final_summary_count.count, 1,
            "Should have 1 summary after completion"
        );

        // Cleanup
        diesel::sql_query("DELETE FROM region_summary WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(summary_id)
            .execute(&mut conn)
            .ok();

        diesel::sql_query("DELETE FROM region_processing_batches WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(batch_id)
            .execute(&mut conn)
            .ok();

        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ Complete workflow simulation passed");
    }

    #[tokio::test]

    async fn test_batch_lifecycle_transitions() {
        let mut conn = get_db_connection();

        // Create test region
        let region_uuid = Uuid::new_v4();
        let region_id = (rand::random::<u16>() as i32) + 10000;
        diesel::sql_query("INSERT INTO region_mapping (id, region_id, name) VALUES ($1, $2, $3)")
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .bind::<diesel::sql_types::Int4, _>(region_id)
            .bind::<diesel::sql_types::Text, _>(&format!("Lifecycle Test Region {}", region_uuid))
            .execute(&mut conn)
            .expect("Failed to insert test region");

        let batch_id = Uuid::new_v4();

        // Test all state transitions
        let states = ["collecting", "ready", "processing", "completed"];

        // Create batch (region_id is UUID FK to region_mapping.id)
        diesel::sql_query(
            "INSERT INTO region_processing_batches 
             (id, region_id, status, expected_task_count, created_at) 
             VALUES ($1, $2, $3, 1, NOW())",
        )
        .bind::<diesel::sql_types::Uuid, _>(batch_id)
        .bind::<diesel::sql_types::Uuid, _>(region_uuid)
        .bind::<diesel::sql_types::Text, _>(states[0])
        .execute(&mut conn)
        .expect("Failed to create batch");

        // Transition through each state
        for state in &states[1..] {
            diesel::sql_query("UPDATE region_processing_batches SET status = $1 WHERE id = $2")
                .bind::<diesel::sql_types::Text, _>(state)
                .bind::<diesel::sql_types::Uuid, _>(batch_id)
                .execute(&mut conn)
                .unwrap_or_else(|_| panic!("Failed to transition to {}", state));

            let batch =
                diesel::sql_query("SELECT id, status FROM region_processing_batches WHERE id = $1")
                    .bind::<diesel::sql_types::Uuid, _>(batch_id)
                    .get_result::<BatchRow>(&mut conn)
                    .expect("Failed to query batch");

            assert_eq!(&batch.status, state);
        }

        // Cleanup
        diesel::sql_query("DELETE FROM region_processing_batches WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(batch_id)
            .execute(&mut conn)
            .ok();

        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ Batch lifecycle transitions work correctly");
    }
}

// -----------------------------------------------------------------------------
// Task 3.6 — HTTP integration tests for endpoints added in PR #69
// -----------------------------------------------------------------------------
//
// These tests build the real axum `Router` via
// `OrchServer::new(Orch::new(Arc::new(OrchServices::new(Arc::new(OrchInfra::new())))))
//     .into_router()`
// and drive requests through `tower::ServiceExt::oneshot`. The router uses the
// production infra against the docker-compose test stack (Postgres on 5433,
// Redis on 6380). Unlike the DB-only tests above, these exercise the full
// request → handler → app → services → infra chain.
//
// New endpoints covered (see plans/2026-04-20-pr69-max-test-coverage-v1.md):
//   * POST /orch/api/pipeline/trigger     (per-phase opt-in)
//   * GET  /orch/dev/api/redis-stats      (happy path + Redis-down degraded)
//   * GET  /orch/dev/api/system-stats     (dev dashboard panel)
//   * GET  /orch/dev/api/summary-freshness (dev dashboard panel)
//
// Gated by `RUN_INTEGRATION_TESTS=1` so unit-test runs without the compose
// stack don't fail. Requires DATABASE_URL=postgres://.../test_db on :5433 and
// REDIS_URL=redis://localhost:6380. MUST be run with --test-threads=1 because
// the Redis-down test mutates process-global env.
mod http_handler_tests {
    use super::*;
    use api::Orch;
    use app::Services;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use diesel::prelude::*;
    use http_body_util::BodyExt;
    use infra::OrchInfra;
    use server::OrchServer;
    use services::OrchServices;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn should_run() -> bool {
        env::var("RUN_INTEGRATION_TESTS").is_ok()
    }

    /// Build the production router against a fresh `OrchInfra`. Each call
    /// constructs a brand-new infra instance so the Redis `OnceCell` is
    /// uncached — essential for the Redis-down test which manipulates
    /// `REDIS_URL` across calls.
    fn build_router() -> Router {
        let infra = Arc::new(OrchInfra::new());
        let services: Arc<OrchServices<OrchInfra>> = Arc::new(OrchServices::new(infra));
        let api = Arc::new(Orch::new(services));
        let orch_server = OrchServer::new(api);
        orch_server.into_router()
    }

    async fn read_body_json(resp: axum::response::Response) -> serde_json::Value {
        let body_bytes = resp
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        serde_json::from_slice::<serde_json::Value>(&body_bytes).unwrap_or_else(|e| {
            panic!(
                "response body is not JSON: {e}; raw = {:?}",
                String::from_utf8_lossy(&body_bytes)
            )
        })
    }

    async fn post_json(router: &Router, path: &str, body: serde_json::Value) -> axum::response::Response {
        let req = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build request");
        router
            .clone()
            .oneshot(req)
            .await
            .expect("router oneshot failed")
    }

    async fn get(router: &Router, path: &str) -> axum::response::Response {
        let req = Request::builder()
            .method(Method::GET)
            .uri(path)
            .body(Body::empty())
            .expect("build request");
        router
            .clone()
            .oneshot(req)
            .await
            .expect("router oneshot failed")
    }

    // -- /orch/health (smoke) -------------------------------------------------

    #[tokio::test]
    async fn health_returns_ok() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = get(&router, "/orch/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(body["status"], "ok");
        println!("✅ /orch/health returns 200 ok");
    }

    // -- POST /orch/api/pipeline/trigger -------------------------------------

    /// Empty body → every phase flag defaults to `false` → every phase is
    /// skipped → response fields stay `None` and `errors` is empty.
    /// Exercises: route table, Json extractor, #[serde(default)] on every
    /// PipelineTriggerRequest field, trigger_pipeline control flow.
    #[tokio::test]
    async fn pipeline_trigger_empty_body_is_noop() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = post_json(&router, "/orch/api/pipeline/trigger", serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::OK, "empty body should be 200");
        let body = read_body_json(resp).await;

        // Every phase was skipped → every result slot is null.
        assert!(body["reset_queries_deleted"].is_null());
        assert!(body["generate_queries_result"].is_null());
        assert!(body["discover_papers_result"].is_null());
        assert!(body["ensure_workers_ok"].is_null());

        let errors = body["errors"].as_array().expect("errors array");
        assert!(
            errors.is_empty(),
            "no phases ran, so no errors expected, got {errors:?}"
        );
        println!("✅ POST /pipeline/trigger {{}} is a no-op");
    }

    /// Completely missing body (no content-type, no JSON) → axum's Json
    /// extractor returns 400 or 415. Either is fine; we just assert the
    /// handler did NOT return 200. Guards against accidental `#[derive(Default)]`
    /// + `Option<Json<...>>` regressions that would make the endpoint
    /// accept any input.
    #[tokio::test]
    async fn pipeline_trigger_malformed_body_rejected() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/orch/api/pipeline/trigger")
            .header("content-type", "application/json")
            .body(Body::from("not json at all"))
            .unwrap();
        let resp = router.oneshot(req).await.expect("oneshot");
        assert_ne!(
            resp.status(),
            StatusCode::OK,
            "malformed JSON must not return 200"
        );
        // Axum's Json extractor maps body-decode failures to 400 by default.
        assert!(
            resp.status().is_client_error(),
            "expected 4xx for malformed JSON body, got {}",
            resp.status()
        );
        println!("✅ POST /pipeline/trigger rejects malformed JSON");
    }

    /// `ensure_workers: true` with no downstream fetcher service (or an
    /// unreachable one) → the phase fails but the endpoint still returns
    /// 200 with the error surfaced inside `errors` and `ensure_workers_ok
    /// = false`. This exercises the "per-phase failure is swallowed"
    /// contract documented on `trigger_pipeline`.
    ///
    /// We set `FETCHER_HTTP_ADDR` to an unreachable port BEFORE building
    /// the router so the lazy http client tries and fails quickly.
    /// Wrapped in a 30s guardrail — fetcher calls on an unreachable port
    /// should RST immediately on localhost.
    #[tokio::test]
    async fn pipeline_trigger_ensure_workers_returns_200_even_on_failure() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }

        // Point fetcher at a port nothing is listening on so ensure_workers
        // fails fast with a TCP RST instead of hanging on a long connect
        // timeout. Restore the original value after the test so later tests
        // in the same process aren't affected.
        let original_fetcher = env::var("FETCHER_HTTP_ADDR").ok();
        // SAFETY: edition 2024; see redis_stats_degraded_when_unreachable.
        unsafe {
            env::set_var("FETCHER_HTTP_ADDR", "http://127.0.0.1:1");
        }

        let router = build_router();
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            post_json(
                &router,
                "/orch/api/pipeline/trigger",
                serde_json::json!({"ensure_workers": true}),
            ),
        )
        .await;

        // Restore env before assertions.
        unsafe {
            match original_fetcher {
                Some(v) => env::set_var("FETCHER_HTTP_ADDR", v),
                None => env::remove_var("FETCHER_HTTP_ADDR"),
            }
        }

        let resp = resp.expect("pipeline_trigger ensure_workers blocked for >30s");

        // Regardless of fetcher availability the HTTP status MUST be 200 —
        // errors are returned inline, not as a 5xx.
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "per-phase failure must NOT produce a 5xx"
        );
        let body = read_body_json(resp).await;

        // Other phases must remain unexecuted.
        assert!(body["reset_queries_deleted"].is_null());
        assert!(body["generate_queries_result"].is_null());
        assert!(body["discover_papers_result"].is_null());

        // ensure_workers_ok is either Some(true) on success or Some(false) on
        // failure, never None (the phase was requested).
        let ok = body["ensure_workers_ok"].as_bool();
        assert!(
            ok.is_some(),
            "ensure_workers phase ran, so ensure_workers_ok must be set"
        );

        // `errors` array shape invariant: each entry is a string.
        let errors = body["errors"].as_array().expect("errors array");
        for e in errors {
            assert!(e.is_string(), "every error entry must be a string");
        }

        // Cross-check: if the phase failed, there must be exactly one error
        // prefixed "ensure_workers:".
        if ok == Some(false) {
            assert!(
                errors.iter().any(|e| e.as_str().unwrap_or("").starts_with("ensure_workers:")),
                "failed ensure_workers must record an error with the phase prefix"
            );
        }
        println!(
            "✅ POST /pipeline/trigger {{ensure_workers: true}} returns 200 + well-formed body"
        );
    }

    /// Discover-papers + generate-queries combo. Either can fail depending on
    /// upstream reachability; the endpoint must still return 200 with the
    /// errors surfaced inline. This also covers the "multiple phases in one
    /// request" code path.
    #[tokio::test]
    async fn pipeline_trigger_combo_discover_and_generate() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = post_json(
            &router,
            "/orch/api/pipeline/trigger",
            serde_json::json!({
                "generate_queries": true,
                "discover_papers": true,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;

        // Both phases were requested — each yields either a result tuple or
        // an error line; either way the paired slot is populated in some way.
        // Specifically: success fills the `*_result` field; failure fills
        // nothing AND adds one entry to `errors`. So the invariant is:
        //   result OR errors-prefixed-with-phase-name.
        let errors: Vec<String> = body["errors"]
            .as_array()
            .expect("errors array")
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect();

        let gen_result_present = !body["generate_queries_result"].is_null();
        let gen_err_present = errors.iter().any(|e| e.starts_with("generate_queries:"));
        assert!(
            gen_result_present || gen_err_present,
            "generate_queries phase must produce a result OR an error; got neither. body={body:?}"
        );

        let disc_result_present = !body["discover_papers_result"].is_null();
        let disc_err_present = errors.iter().any(|e| e.starts_with("discover_papers:"));
        assert!(
            disc_result_present || disc_err_present,
            "discover_papers phase must produce a result OR an error; got neither. body={body:?}"
        );

        // Phases NOT requested must remain None.
        assert!(body["reset_queries_deleted"].is_null());
        assert!(body["ensure_workers_ok"].is_null());
        println!("✅ POST /pipeline/trigger combo phases respond cleanly");
    }

    // -- GET /orch/dev/api/redis-stats ---------------------------------------

    /// Happy path: Redis is up (docker-compose exposes it on :6380). Handler
    /// must return 200 with `connected: true` and a populated
    /// `keys_by_prefix` array (one entry per known prefix pattern).
    #[tokio::test]
    async fn redis_stats_happy_path() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = get(&router, "/orch/dev/api/redis-stats").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;

        assert_eq!(
            body["connected"], serde_json::Value::Bool(true),
            "happy-path Redis must be reported as connected; body={body:?}"
        );
        assert!(
            body["error"].is_null(),
            "no error expected on happy path, got {}",
            body["error"]
        );

        // keys_by_prefix is populated only when connected.
        let prefixes = body["keys_by_prefix"].as_array().expect("keys_by_prefix");
        assert!(
            !prefixes.is_empty(),
            "when connected, keys_by_prefix must list the known patterns"
        );
        for entry in prefixes {
            assert!(entry["pattern"].is_string());
            assert!(entry["description"].is_string());
            assert!(entry["count"].is_number());
        }

        // server_version is populated by INFO parsing; non-empty on real Redis.
        assert!(
            body["server_version"].as_str().unwrap_or("").len() > 0,
            "real redis should report a version string"
        );
        println!("✅ /dev/api/redis-stats happy path returns connected: true");
    }

    /// Redis-down degraded path: point `REDIS_URL` at an unreachable host so
    /// DNS resolution fails fast. Build a NEW router (fresh `OrchInfra`,
    /// so `OnceCell` is empty and the connection attempt will use the bad
    /// URL). The production `cache_stats` impl is documented to return
    /// `Ok(RedisStats { connected: false, error: Some(...), .. })` when it
    /// can't connect, so the HTTP layer must surface 200 + `connected:
    /// false`.
    ///
    /// This mutates process-global env and MUST run under
    /// `--test-threads=1` (already enforced by the outer test config).
    /// The test is wrapped in a `tokio::time::timeout` as a guardrail so
    /// a misconfigured redis-crate retry policy can't hang CI.
    #[tokio::test]
    async fn redis_stats_degraded_when_unreachable() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        // Save original and point at a malformed URL so `redis::Client::open`
        // fails synchronously during URL parsing — avoids the slow
        // `ConnectionManager::new` retry path (which, at the default 6
        // retries × backoff × TCP connect timeout, can run into minutes
        // on a host that can't resolve).
        let original = env::var("REDIS_URL").ok();
        // SAFETY: edition 2024 marks `set_var` as unsafe; we manipulate
        // process-global env in a serialised test run (--test-threads=1).
        unsafe {
            env::set_var("REDIS_URL", "not-a-valid-redis-url");
        }

        // Build a FRESH router so its OrchInfra's OnceCell picks up the new
        // URL on first connect.
        let router = build_router();

        // 10s guardrail. With a malformed URL `redis::Client::open` fails
        // synchronously, so this should complete in well under a second.
        // If we ever exceed this bound, fail loudly rather than hang CI.
        let resp = match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            get(&router, "/orch/dev/api/redis-stats"),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => {
                unsafe {
                    match original {
                        Some(v) => env::set_var("REDIS_URL", v),
                        None => env::remove_var("REDIS_URL"),
                    }
                }
                panic!(
                    "redis-stats handler blocked for >30s when Redis was unreachable — \
                     the `Ok(degraded)` contract on `cache_stats` is broken"
                );
            }
        };

        // Restore env before any assertion so a failure doesn't poison
        // downstream tests.
        unsafe {
            match original {
                Some(v) => env::set_var("REDIS_URL", v),
                None => env::remove_var("REDIS_URL"),
            }
        }

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "contract: redis-stats NEVER returns 5xx even when Redis is down"
        );
        let body = read_body_json(resp).await;
        assert_eq!(
            body["connected"],
            serde_json::Value::Bool(false),
            "expected connected: false when Redis is unreachable; body={body:?}"
        );
        assert!(
            body["error"].is_string(),
            "error message must be populated when connection fails"
        );
        // Counters must be zeroed on the degraded shape.
        assert_eq!(body["total_keys"].as_u64().unwrap_or(u64::MAX), 0);
        assert_eq!(
            body["keys_by_prefix"].as_array().expect("array").len(),
            0,
            "keys_by_prefix must be empty when disconnected"
        );
        println!("✅ /dev/api/redis-stats degrades gracefully when Redis is unreachable");
    }

    // -- GET /orch/dev/api/system-stats --------------------------------------

    /// Dev-dashboard panel: aggregate of fetch_tasks, batches, queries,
    /// papers, summaries. Must return 200 with the well-formed shape.
    #[tokio::test]
    async fn dev_system_stats_returns_populated_snapshot() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }

        // Seed a region + a couple of queries so the aggregate is non-zero.
        let mut conn = get_db_connection();
        let region_uuid = Uuid::new_v4();
        let region_id = (rand::random::<u16>() as i32) + 10000;
        diesel::sql_query("INSERT INTO region_mapping (id, region_id, name) VALUES ($1, $2, $3)")
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .bind::<diesel::sql_types::Int4, _>(region_id)
            .bind::<diesel::sql_types::Text, _>(&format!("Dev Stats Region {}", region_uuid))
            .execute(&mut conn)
            .expect("insert region");

        let query_ids: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
        for q_id in &query_ids {
            diesel::sql_query(
                "INSERT INTO region_queries (id, region_id, query_text) VALUES ($1, $2, $3)",
            )
            .bind::<diesel::sql_types::Uuid, _>(*q_id)
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .bind::<diesel::sql_types::Text, _>(&format!("test query {}", q_id))
            .execute(&mut conn)
            .expect("insert region_queries row");
        }

        let router = build_router();
        let resp = get(&router, "/orch/dev/api/system-stats").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;

        // Shape invariants
        assert!(body["fetch_tasks_by_status"].is_array());
        assert!(body["batches_by_status"].is_array());
        assert!(body["query_distribution"].is_array());
        assert!(body["total_queries"].is_number());
        assert!(body["regions_with_queries"].is_number());
        assert!(body["total_papers"].is_number());
        assert!(body["total_summaries"].is_number());
        assert!(body["timestamp"].is_string());

        // total_queries and regions_with_queries must include our seeded rows.
        let total_queries = body["total_queries"].as_i64().expect("total_queries");
        assert!(
            total_queries >= 2,
            "global total_queries={total_queries} should include our 2 inserts"
        );
        let regions_with_queries =
            body["regions_with_queries"].as_i64().expect("regions_with_queries");
        assert!(
            regions_with_queries >= 1,
            "regions_with_queries={regions_with_queries} should include our region"
        );

        // Cleanup
        for q_id in &query_ids {
            diesel::sql_query("DELETE FROM region_queries WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(*q_id)
                .execute(&mut conn)
                .ok();
        }
        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ /dev/api/system-stats returns well-formed snapshot");
    }

    // -- GET /orch/dev/api/summary-freshness ---------------------------------

    /// Dev-dashboard panel: fresh/stale/no_summary buckets across all regions.
    /// Seed one healthy region (is_active + non-empty summary, recent) and
    /// assert `fresh >= 1`, `staleness_days` is passed through, and the
    /// response shape is complete.
    #[tokio::test]
    async fn dev_summary_freshness_returns_buckets() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }

        let mut conn = get_db_connection();
        let region_uuid = Uuid::new_v4();
        let region_id = (rand::random::<u16>() as i32) + 10000;
        diesel::sql_query("INSERT INTO region_mapping (id, region_id, name) VALUES ($1, $2, $3)")
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .bind::<diesel::sql_types::Int4, _>(region_id)
            .bind::<diesel::sql_types::Text, _>(&format!("Freshness Region {}", region_uuid))
            .execute(&mut conn)
            .expect("insert region");

        // Healthy summary: is_active=true, non-empty summary, created NOW().
        let batch_id = Uuid::new_v4();
        let summary_id = Uuid::new_v4();
        diesel::sql_query(
            "INSERT INTO region_summary \
             (id, region_id, name, summary, is_active, batch_id, created_at) \
             VALUES ($1, $2, $3, $4, TRUE, $5, NOW())",
        )
        .bind::<diesel::sql_types::Uuid, _>(summary_id)
        .bind::<diesel::sql_types::Int4, _>(region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Freshness Region {}", region_uuid))
        .bind::<diesel::sql_types::Text, _>("fresh content")
        .bind::<diesel::sql_types::Uuid, _>(batch_id)
        .execute(&mut conn)
        .expect("insert summary");

        let router = build_router();
        let resp = get(&router, "/orch/dev/api/summary-freshness").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;

        // Shape invariants
        assert!(body["fresh"].is_number(), "fresh must be numeric");
        assert!(body["stale"].is_number(), "stale must be numeric");
        assert!(body["no_summary"].is_number(), "no_summary must be numeric");
        assert!(
            body["staleness_days"].is_number(),
            "staleness_days must be numeric"
        );

        // Regression: our just-seeded region MUST contribute to `fresh`.
        let fresh = body["fresh"].as_i64().expect("fresh");
        assert!(
            fresh >= 1,
            "global fresh={fresh} should include our freshly-seeded region"
        );

        // staleness_days must be positive (default implementation uses 7d).
        let staleness_days = body["staleness_days"].as_i64().expect("staleness_days");
        assert!(
            staleness_days > 0,
            "staleness_days={staleness_days} should be a positive window"
        );

        // Cleanup
        diesel::sql_query("DELETE FROM region_summary WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(summary_id)
            .execute(&mut conn)
            .ok();
        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ /dev/api/summary-freshness returns bucket counts including fresh region");
    }

    // -- Negative: unknown route -------------------------------------------

    #[tokio::test]
    async fn unknown_route_returns_404() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = get(&router, "/orch/api/does-not-exist").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        println!("✅ unknown route returns 404");
    }

    /// `Services` must not be imported as unused; touch the trait marker so
    /// the import stays grounded in case rustc lint-cleans the module.
    #[allow(dead_code)]
    fn _services_bound<S: Services>(_s: &S) {}
}

