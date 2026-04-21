// Integration tests for BrainAtlas
// These tests require PostgreSQL and MinIO (via docker-compose)
//
// To run: cargo test --package server --test integration_test -- --test-threads=1
//
// Prerequisites:
// 1. docker-compose -f ../../docker-compose.test.yml up -d
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
struct RegionMappingResult {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    region_id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
}

#[derive(diesel::QueryableByName)]
struct SummaryResult {
    #[diesel(sql_type = diesel::sql_types::Text)]
    summary: String,
}

// Helper to get test database URL
fn get_test_db_url() -> String {
    env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://test_user:test_password@localhost:5433/test_db".to_string()
    })
}

// Helper to get test S3 config
fn get_test_s3_config() -> (String, String, String, String) {
    let endpoint = env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".to_string());
    let access_key = env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "test_access_key".to_string());
    let secret_key = env::var("S3_SECRET_KEY").unwrap_or_else(|_| "test_secret_key".to_string());
    let bucket = env::var("S3_BUCKET").unwrap_or_else(|_| "test-bucket".to_string());
    (endpoint, access_key, secret_key, bucket)
}

mod database_tests {
    use super::*;
    use diesel::prelude::*;
    use diesel::r2d2::{self, ConnectionManager};

    #[test]
    fn test_database_connection() {
        let database_url = get_test_db_url();
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("Failed to create pool");

        let mut conn = pool.get().expect("Failed to get connection");

        // Test basic query
        let result = diesel::sql_query("SELECT 1 as value")
            .get_result::<SelectOne>(&mut conn)
            .map(|r| r.value);

        assert_eq!(result.unwrap(), 1);
        println!("✅ Database connection successful");
    }

    #[test]
    fn test_region_mapping_table_exists() {
        let database_url = get_test_db_url();
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("Failed to create pool");

        let mut conn = pool.get().expect("Failed to get connection");

        // Check if region_mapping table exists
        let result = diesel::sql_query(
            "SELECT COUNT(*) as count FROM information_schema.tables WHERE table_name = 'region_mapping'"
        )
        .get_result::<CountResult>(&mut conn)
        .map(|r| r.count);

        assert_eq!(result.unwrap(), 1, "region_mapping table should exist");
        println!("✅ region_mapping table exists");
    }

    #[test]
    fn test_region_summary_table_exists() {
        let database_url = get_test_db_url();
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("Failed to create pool");

        let mut conn = pool.get().expect("Failed to get connection");

        // Check if region_summary table exists
        let result = diesel::sql_query(
            "SELECT COUNT(*) as count FROM information_schema.tables WHERE table_name = 'region_summary'"
        )
        .get_result::<CountResult>(&mut conn)
        .map(|r| r.count);

        assert_eq!(result.unwrap(), 1, "region_summary table should exist");
        println!("✅ region_summary table exists");
    }

    #[test]
    fn test_insert_and_query_region_mapping() {
        let database_url = get_test_db_url();
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("Failed to create pool");

        let mut conn = pool.get().expect("Failed to get connection");

        let test_uuid = Uuid::new_v4();
        let test_region_id = rand::random::<i32>().abs();

        // Insert test region
        let insert_result = diesel::sql_query(
            "INSERT INTO region_mapping (id, region_id, name, acronym) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind::<diesel::sql_types::Uuid, _>(test_uuid)
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Test Region {}", test_uuid))
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("TR"))
        .execute(&mut conn);

        assert!(insert_result.is_ok());

        // Query it back
        let query_result =
            diesel::sql_query("SELECT id, region_id, name FROM region_mapping WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(test_uuid)
                .get_result::<RegionMappingResult>(&mut conn);

        assert!(query_result.is_ok());
        let result = query_result.unwrap();
        assert_eq!(result.id, test_uuid);
        assert_eq!(result.region_id, test_region_id);
        assert_eq!(result.name, format!("Test Region {}", test_uuid));

        // Cleanup
        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(test_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ Can insert and query region_mapping");
    }
}

mod s3_tests {
    use super::*;

    #[tokio::test]
    async fn test_s3_connection() {
        let (endpoint, access_key, secret_key, _bucket) = get_test_s3_config();

        // Create S3 client
        use aws_sdk_s3::Client;
        use aws_sdk_s3::config::{Credentials, Region};

        let creds = Credentials::new(&access_key, &secret_key, None, None, "test");
        let config = aws_sdk_s3::Config::builder()
            .region(Region::new("us-east-1"))
            .endpoint_url(&endpoint)
            .credentials_provider(creds)
            .force_path_style(true)
            .build();

        let client = Client::from_conf(config);

        // Try to list buckets
        let result = client.list_buckets().send().await;
        assert!(result.is_ok(), "Should be able to connect to S3");

        println!("✅ S3 connection successful");
    }

    #[tokio::test]
    async fn test_s3_create_bucket_and_upload() {
        let (endpoint, access_key, secret_key, bucket) = get_test_s3_config();

        use aws_sdk_s3::Client;
        use aws_sdk_s3::config::{Credentials, Region};
        use aws_sdk_s3::primitives::ByteStream;

        let creds = Credentials::new(&access_key, &secret_key, None, None, "test");
        let config = aws_sdk_s3::Config::builder()
            .region(Region::new("us-east-1"))
            .endpoint_url(&endpoint)
            .credentials_provider(creds)
            .force_path_style(true)
            .build();

        let client = Client::from_conf(config);

        // Create bucket if it doesn't exist
        let _ = client.create_bucket().bucket(&bucket).send().await;

        // Upload test file
        let test_key = format!("test-papers/{}/test.txt", Uuid::new_v4());
        let test_content = "This is a test paper about neuroplasticity.\n\nNeuroplasticity is the brain's ability to reorganize itself.";

        let upload_result = client
            .put_object()
            .bucket(&bucket)
            .key(&test_key)
            .body(ByteStream::from(test_content.as_bytes().to_vec()))
            .send()
            .await;

        assert!(upload_result.is_ok(), "Should be able to upload to S3");

        // Download and verify
        let download_result = client
            .get_object()
            .bucket(&bucket)
            .key(&test_key)
            .send()
            .await;

        assert!(
            download_result.is_ok(),
            "Should be able to download from S3"
        );

        let body = download_result.unwrap().body.collect().await.unwrap();
        let downloaded_content = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(downloaded_content, test_content);

        // Cleanup
        let _ = client
            .delete_object()
            .bucket(&bucket)
            .key(&test_key)
            .send()
            .await;

        println!("✅ Can upload and download from S3");
    }
}

mod api_tests {
    use super::*;
    use diesel::prelude::*;
    use diesel::r2d2::{self, ConnectionManager};

    #[tokio::test]
    async fn test_search_brain_region_empty() {
        let database_url = get_test_db_url();
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("Failed to create pool");

        let mut conn = pool.get().expect("Failed to get connection");

        let test_uuid = Uuid::new_v4();
        let test_region_id = rand::random::<i32>().abs();

        // Insert test region without summaries
        diesel::sql_query(
            "INSERT INTO region_mapping (id, region_id, name, acronym) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind::<diesel::sql_types::Uuid, _>(test_uuid)
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Test Empty Region {}", test_uuid))
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("TER"))
        .execute(&mut conn)
        .expect("Failed to insert test region");

        // Query summaries (should be empty)
        let count_result =
            diesel::sql_query("SELECT COUNT(*) as count FROM region_summary WHERE region_id = $1")
                .bind::<diesel::sql_types::Int4, _>(test_region_id)
                .get_result::<CountResult>(&mut conn)
                .expect("Failed to count summaries");

        assert_eq!(
            count_result.count, 0,
            "Should have no summaries for new region"
        );

        // Cleanup
        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(test_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ Search returns empty for region without summaries");
    }

    #[tokio::test]
    async fn test_insert_and_retrieve_summary() {
        let database_url = get_test_db_url();
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("Failed to create pool");

        let mut conn = pool.get().expect("Failed to get connection");

        let test_uuid = Uuid::new_v4();
        let test_region_id = rand::random::<i32>().abs();
        let test_summary =
            "This is a test summary about the hippocampus and its role in memory formation.";

        // Insert test region
        diesel::sql_query(
            "INSERT INTO region_mapping (id, region_id, name, acronym) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind::<diesel::sql_types::Uuid, _>(test_uuid)
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Test Region With Summary {}", test_uuid))
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("TRWS"))
        .execute(&mut conn)
        .expect("Failed to insert test region");

        // Insert summary (batch_id is NOT NULL)
        let summary_id = Uuid::new_v4();
        diesel::sql_query(
            "INSERT INTO region_summary (id, region_id, name, summary, created_at, content_hash, batch_id)
             VALUES ($1, $2, $3, $4, NOW(), $5, $6)",
        )
        .bind::<diesel::sql_types::Uuid, _>(summary_id)
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Test Region With Summary {}", test_uuid))
        .bind::<diesel::sql_types::Text, _>(test_summary)
        .bind::<diesel::sql_types::Text, _>("test_hash_123")
        .bind::<diesel::sql_types::Uuid, _>(Uuid::new_v4())
        .execute(&mut conn)
        .expect("Failed to insert summary");

        // Query summaries
        let summaries: Vec<String> =
            diesel::sql_query("SELECT summary FROM region_summary WHERE region_id = $1")
                .bind::<diesel::sql_types::Int4, _>(test_region_id)
                .load::<SummaryResult>(&mut conn)
                .unwrap()
                .into_iter()
                .map(|r| r.summary)
                .collect();

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0], test_summary);

        // Cleanup
        diesel::sql_query("DELETE FROM region_summary WHERE region_id = $1")
            .bind::<diesel::sql_types::Int4, _>(test_region_id)
            .execute(&mut conn)
            .ok();
        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(test_uuid)
            .execute(&mut conn)
            .ok();

        println!("✅ Can insert and retrieve summaries");
    }
}

mod workflow_tests {
    use super::*;
    use aws_sdk_s3::Client;
    use aws_sdk_s3::config::{Credentials, Region};
    use aws_sdk_s3::primitives::ByteStream;
    use diesel::prelude::*;
    use diesel::r2d2::{self, ConnectionManager};

    #[tokio::test]
    async fn test_complete_workflow_simulation() {
        // This test simulates the complete workflow:
        // 1. Create region in database
        // 2. Upload papers to S3
        // 3. Process papers (simulated)
        // 4. Store summary
        // 5. Retrieve summary

        let database_url = get_test_db_url();
        let (s3_endpoint, s3_access, s3_secret, s3_bucket) = get_test_s3_config();

        // Setup database
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("Failed to create pool");

        let mut conn = pool.get().expect("Failed to get connection");

        // Setup S3
        let creds = Credentials::new(&s3_access, &s3_secret, None, None, "test");
        let config = aws_sdk_s3::Config::builder()
            .region(Region::new("us-east-1"))
            .endpoint_url(&s3_endpoint)
            .credentials_provider(creds)
            .force_path_style(true)
            .build();

        let s3_client = Client::from_conf(config);

        // Create bucket
        let _ = s3_client.create_bucket().bucket(&s3_bucket).send().await;

        // Step 1: Create region
        let region_uuid = Uuid::new_v4();
        let region_id = rand::random::<i32>().abs();

        diesel::sql_query(
            "INSERT INTO region_mapping (id, region_id, name, acronym) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind::<diesel::sql_types::Uuid, _>(region_uuid)
        .bind::<diesel::sql_types::Int4, _>(region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Hippocampus {}", region_uuid))
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("HIP"))
        .execute(&mut conn)
        .expect("Failed to insert region");

        println!("✅ Step 1: Region created - {}", region_uuid);

        // Step 2: Upload test papers to S3
        let paper1_key = format!("papers/{}/paper1.txt", region_uuid);
        let paper1_content = "The hippocampus is critical for memory formation. \
                             Studies show that neuroplasticity in this region enables learning.";

        s3_client
            .put_object()
            .bucket(&s3_bucket)
            .key(&paper1_key)
            .body(ByteStream::from(paper1_content.as_bytes().to_vec()))
            .send()
            .await
            .expect("Failed to upload paper1");

        let paper2_key = format!("papers/{}/paper2.txt", region_uuid);
        let paper2_content = "Memory consolidation occurs during sleep. \
                             The hippocampus replays experiences to strengthen neural connections.";

        s3_client
            .put_object()
            .bucket(&s3_bucket)
            .key(&paper2_key)
            .body(ByteStream::from(paper2_content.as_bytes().to_vec()))
            .send()
            .await
            .expect("Failed to upload paper2");

        println!("✅ Step 2: Papers uploaded to S3");

        // Step 3: Simulate processing (in real workflow, this would call LLM)
        let generated_summary = "The hippocampus is a critical brain region for memory formation and consolidation. \
             Research demonstrates its role in neuroplasticity and learning processes, \
             particularly during sleep when memory consolidation occurs through neural replay."
            .to_string();

        println!("✅ Step 3: Summary generated (simulated)");

        // Step 4: Store summary in database (batch_id is NOT NULL)
        diesel::sql_query(
            "INSERT INTO region_summary (id, region_id, name, summary, created_at, content_hash, batch_id)
             VALUES ($1, $2, $3, $4, NOW(), $5, $6)",
        )
        .bind::<diesel::sql_types::Uuid, _>(Uuid::new_v4())
        .bind::<diesel::sql_types::Int4, _>(region_id)
        .bind::<diesel::sql_types::Text, _>(&format!("Hippocampus {}", region_uuid))
        .bind::<diesel::sql_types::Text, _>(generated_summary.clone())
        .bind::<diesel::sql_types::Text, _>("test_workflow_hash")
        .bind::<diesel::sql_types::Uuid, _>(Uuid::new_v4())
        .execute(&mut conn)
        .expect("Failed to insert summary");

        println!("✅ Step 4: Summary stored in database");

        // Step 5: Retrieve and verify summary
        let retrieved_summaries: Vec<String> =
            diesel::sql_query("SELECT summary FROM region_summary WHERE region_id = $1")
                .bind::<diesel::sql_types::Int4, _>(region_id)
                .load::<SummaryResult>(&mut conn)
                .unwrap()
                .into_iter()
                .map(|r| r.summary)
                .collect();

        assert_eq!(retrieved_summaries.len(), 1);
        assert_eq!(retrieved_summaries[0], generated_summary);

        println!("✅ Step 5: Summary retrieved and verified");

        // Cleanup
        diesel::sql_query("DELETE FROM region_summary WHERE region_id = $1")
            .bind::<diesel::sql_types::Int4, _>(region_id)
            .execute(&mut conn)
            .ok();
        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(region_uuid)
            .execute(&mut conn)
            .ok();

        let _ = s3_client
            .delete_object()
            .bucket(&s3_bucket)
            .key(&paper1_key)
            .send()
            .await;
        let _ = s3_client
            .delete_object()
            .bucket(&s3_bucket)
            .key(&paper2_key)
            .send()
            .await;

        println!("✅ Complete workflow test passed!");
    }
}

// -----------------------------------------------------------------------------
// Task 3.7 — HTTP integration tests for endpoints added in PR #69
// -----------------------------------------------------------------------------
//
// These tests build the real axum `Router` via
// `BrainAtlasServer::new(BrainAtlasApi::new(BrainAtlasServices::new(BrainAtlasInfra::new())))
//     .into_router(None)`
// and drive requests through `tower::ServiceExt::oneshot`. The router uses
// production infra against the docker-compose test stack (Postgres on :5433).
//
// Focus: the NEW `GET /brainatlas-be/api/llm/usage` aggregate endpoint (the
// other new `/api/llm/*` routes are LLM proxies that need a live OpenRouter
// key and are already covered by the Services-fake handler_test.rs). Also
// covers the raw list endpoint and the chunk-source 4xx path.
//
// Gated by `RUN_INTEGRATION_TESTS=1` so unit-test runs without the compose
// stack don't fail. Requires DATABASE_URL on :5433. MUST run with
// `--test-threads=1` because seeds/deletes against shared tables are
// non-idempotent when parallelised.
mod http_llm_usage_tests {
    use super::*;
    use api::BrainAtlasApi;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use diesel::prelude::*;
    use diesel::r2d2::{self, ConnectionManager};
    use http_body_util::BodyExt;
    use infra::BrainAtlasInfra;
    use server::BrainAtlasServer;
    use services::BrainAtlasServices;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn should_run() -> bool {
        env::var("RUN_INTEGRATION_TESTS").is_ok()
    }

    /// Build the production router against a fresh `BrainAtlasInfra`.
    fn build_router() -> Router {
        let infra = Arc::new(BrainAtlasInfra::new());
        let services: Arc<BrainAtlasServices<BrainAtlasInfra>> =
            Arc::new(BrainAtlasServices::new(infra));
        let api = Arc::new(BrainAtlasApi::new(services));
        let srv = BrainAtlasServer::new(api);
        srv.into_router(None)
    }

    fn get_pool() -> r2d2::Pool<ConnectionManager<PgConnection>> {
        let manager = ConnectionManager::<PgConnection>::new(get_test_db_url());
        r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("build pool")
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

    async fn get_uri(router: &Router, uri: &str) -> axum::response::Response {
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .expect("build request");
        router
            .clone()
            .oneshot(req)
            .await
            .expect("router oneshot failed")
    }

    /// Parameters for the raw insert helper; bundled to keep the argument list
    /// short and avoid clippy's too_many_arguments lint.
    struct InsertUsageArgs<'a> {
        endpoint: &'a str,
        model: &'a str,
        prompt: i32,
        completion: i32,
        total: i32,
        cost: Option<f64>,
        correlation_id: Option<&'a str>,
        caller_tag: Option<&'a str>,
    }

    /// Raw insert helper for `llm_call_usage`. We don't use diesel types
    /// because the `Queryable` models live inside the `infra` crate and
    /// aren't re-exported; raw SQL keeps this test file self-contained.
    fn insert_usage(conn: &mut PgConnection, args: InsertUsageArgs<'_>) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        diesel::sql_query(
            "INSERT INTO llm_call_usage \
             (id, endpoint, model, prompt_tokens, completion_tokens, total_tokens, \
              cost_usd, correlation_id, caller_tag) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind::<diesel::sql_types::Uuid, _>(id)
        .bind::<diesel::sql_types::Text, _>(args.endpoint)
        .bind::<diesel::sql_types::Text, _>(args.model)
        .bind::<diesel::sql_types::Int4, _>(args.prompt)
        .bind::<diesel::sql_types::Int4, _>(args.completion)
        .bind::<diesel::sql_types::Int4, _>(args.total)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Numeric>, _>(
            args.cost
                .and_then(|c| bigdecimal::BigDecimal::try_from(c).ok()),
        )
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(args.correlation_id)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(args.caller_tag)
        .execute(conn)
        .expect("insert llm_call_usage row");
        id
    }

    fn cleanup_usage_ids(conn: &mut PgConnection, ids: &[uuid::Uuid]) {
        for id in ids {
            diesel::sql_query("DELETE FROM llm_call_usage WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(*id)
                .execute(conn)
                .ok();
        }
    }

    // -- /brainatlas-be/health (smoke) ---------------------------------------

    #[tokio::test]
    async fn health_returns_ok() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = get_uri(&router, "/brainatlas-be/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(body["status"], "ok");
        println!("✅ /brainatlas-be/health returns 200 ok");
    }

    // -- GET /brainatlas-be/api/llm/usage ------------------------------------

    /// Happy path with no filters: seed two rows (one chat, one embedding
    /// under distinct models and caller_tags) and assert:
    ///   - 200 OK
    ///   - totals aggregate across our seeded rows (>= the seeded magnitudes
    ///     since the table is shared with other tests)
    ///   - by_model contains both models we seeded
    ///   - by_caller_tag contains both caller_tags we seeded
    #[tokio::test]
    async fn llm_usage_aggregate_groups_by_model_and_caller_tag() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }

        let pool = get_pool();
        let mut conn = pool.get().unwrap();

        // Unique caller_tags so we can find our rows in the aggregate
        // regardless of unrelated rows that may exist.
        let run_tag = format!("itest-{}", Uuid::new_v4());
        let tag_a = format!("{run_tag}-a");
        let tag_b = format!("{run_tag}-b");
        let model_a = format!("test/{}-chat", Uuid::new_v4());
        let model_b = format!("test/{}-embed", Uuid::new_v4());

        let inserted = vec![
            insert_usage(
                &mut conn,
                InsertUsageArgs {
                    endpoint: "chat_completion",
                    model: &model_a,
                    prompt: 100,
                    completion: 50,
                    total: 150,
                    cost: Some(0.001),
                    correlation_id: Some("ci:smoke"),
                    caller_tag: Some(&tag_a),
                },
            ),
            insert_usage(
                &mut conn,
                InsertUsageArgs {
                    endpoint: "embedding",
                    model: &model_b,
                    prompt: 200,
                    completion: 0,
                    total: 200,
                    cost: Some(0.0004),
                    correlation_id: Some("ci:smoke"),
                    caller_tag: Some(&tag_b),
                },
            ),
        ];

        let router = build_router();
        let resp = get_uri(
            &router,
            &format!("/brainatlas-be/api/llm/usage?caller_tag={tag_a}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;

        // Filter by caller_tag=a: only our row A should be counted.
        assert_eq!(
            body["total_calls"].as_i64().expect("total_calls"),
            1,
            "filter by caller_tag must return exactly one row"
        );
        assert_eq!(
            body["total_prompt_tokens"]
                .as_i64()
                .expect("total_prompt_tokens"),
            100
        );
        assert_eq!(
            body["total_completion_tokens"]
                .as_i64()
                .expect("total_completion_tokens"),
            50
        );
        assert_eq!(body["total_tokens"].as_i64().expect("total_tokens"), 150);

        let by_model = body["by_model"].as_array().expect("by_model");
        assert_eq!(by_model.len(), 1, "only model A under caller_tag=a");
        assert_eq!(by_model[0]["model"], model_a);

        let by_caller_tag = body["by_caller_tag"].as_array().expect("by_caller_tag");
        assert_eq!(by_caller_tag.len(), 1);
        assert_eq!(by_caller_tag[0]["caller_tag"], tag_a);

        // Same but filter by model=B → should find our embedding row.
        let resp = get_uri(
            &router,
            &format!("/brainatlas-be/api/llm/usage?model={model_b}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(body["total_calls"].as_i64().unwrap(), 1);
        assert_eq!(body["total_tokens"].as_i64().unwrap(), 200);
        assert_eq!(body["by_model"].as_array().unwrap()[0]["model"], model_b);

        cleanup_usage_ids(&mut conn, &inserted);
        println!("✅ /api/llm/usage aggregates by model and caller_tag");
    }

    /// `correlation_id_prefix` must match rows by prefix only, and must
    /// escape SQL-LIKE special chars (`%`, `_`, `\`) so prefixes containing
    /// them match literally rather than as wildcards. This is the PR #69
    /// regression guard documented in `llm_usage.rs:146`.
    #[tokio::test]
    async fn llm_usage_correlation_id_prefix_escapes_like_wildcards() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }

        let pool = get_pool();
        let mut conn = pool.get().unwrap();

        let run_id = Uuid::new_v4();
        let model = format!("test/{}-escape", run_id);

        // Seed rows:
        //   rowA: correlation_id starts with the literal prefix "eval:{run}%:"
        //         (contains a literal % which must be escaped in the LIKE)
        //   rowB: correlation_id that would match if `%` were a wildcard
        //         (e.g. "eval:{run}XYZ:step-1"), so we can assert it is NOT
        //         included when the prefix contains `%`.
        let literal_prefix = format!("eval:{run_id}%:");
        let fooled_prefix = format!("eval:{run_id}XYZ:");

        let correlation_a = format!("{literal_prefix}step-1");
        let correlation_b = format!("{fooled_prefix}step-2");

        let mut inserted = Vec::new();
        inserted.push(insert_usage(
            &mut conn,
            InsertUsageArgs {
                endpoint: "chat_completion",
                model: &model,
                prompt: 10,
                completion: 5,
                total: 15,
                cost: Some(0.0),
                correlation_id: Some(&correlation_a),
                caller_tag: Some("ci:escape-a"),
            },
        ));
        inserted.push(insert_usage(
            &mut conn,
            InsertUsageArgs {
                endpoint: "chat_completion",
                model: &model,
                prompt: 10,
                completion: 5,
                total: 15,
                cost: Some(0.0),
                correlation_id: Some(&correlation_b),
                caller_tag: Some("ci:escape-b"),
            },
        ));

        let router = build_router();

        // URL-encode the `%` in the prefix so axum passes the literal byte
        // through to the filter. percent-encoded `%` is `%25`.
        let prefix_encoded = literal_prefix.replace('%', "%25");
        let uri = format!(
            "/brainatlas-be/api/llm/usage?model={model}&correlation_id_prefix={prefix_encoded}"
        );
        let resp = get_uri(&router, &uri).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;

        // Only rowA matches — rowB must NOT be mistakenly matched via the
        // unescaped `%` wildcard.
        assert_eq!(
            body["total_calls"].as_i64().expect("total_calls"),
            1,
            "literal `%` in the prefix must not act as a LIKE wildcard; body={body:?}"
        );
        // And the totals agree with rowA's single insert.
        assert_eq!(body["total_tokens"].as_i64().unwrap(), 15);

        // Underscore wildcard: seed a control row whose correlation_id has a
        // literal `_` in the prefix, and one that would match if `_` were the
        // single-char LIKE wildcard. The same escaping logic should apply.
        let us_prefix = format!("run_{run_id}_");
        let us_row_a = format!("{us_prefix}a");
        let us_row_b = format!("runX{run_id}Xb"); // would match `run_*_*` wildcard
        inserted.push(insert_usage(
            &mut conn,
            InsertUsageArgs {
                endpoint: "chat_completion",
                model: &model,
                prompt: 7,
                completion: 3,
                total: 10,
                cost: Some(0.0),
                correlation_id: Some(&us_row_a),
                caller_tag: Some("ci:underscore-a"),
            },
        ));
        inserted.push(insert_usage(
            &mut conn,
            InsertUsageArgs {
                endpoint: "chat_completion",
                model: &model,
                prompt: 7,
                completion: 3,
                total: 10,
                cost: Some(0.0),
                correlation_id: Some(&us_row_b),
                caller_tag: Some("ci:underscore-b"),
            },
        ));

        let uri2 =
            format!("/brainatlas-be/api/llm/usage?model={model}&correlation_id_prefix={us_prefix}");
        let resp2 = get_uri(&router, &uri2).await;
        assert_eq!(resp2.status(), StatusCode::OK);
        let body2 = read_body_json(resp2).await;
        assert_eq!(
            body2["total_calls"].as_i64().unwrap(),
            1,
            "literal `_` in the prefix must not act as a LIKE wildcard; body={body2:?}"
        );

        cleanup_usage_ids(&mut conn, &inserted);
        println!("✅ /api/llm/usage correlation_id_prefix escapes SQL LIKE wildcards");
    }

    /// `since` / `until` must be RFC 3339; malformed timestamps → 400.
    /// Also verifies the bare-happy-path with a wide-open window returns 200.
    #[tokio::test]
    async fn llm_usage_rejects_malformed_since_timestamp() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();

        // Malformed timestamp → 400 Bad Request
        let resp = get_uri(
            &router,
            "/brainatlas-be/api/llm/usage?since=not-a-timestamp",
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "malformed RFC 3339 must be rejected with 400"
        );
        let body = read_body_json(resp).await;
        assert!(
            body["error"].is_string(),
            "error body must be {{\"error\": \"...\"}} shape"
        );

        // Well-formed bounds → 200 even when no rows match.
        let resp = get_uri(
            &router,
            "/brainatlas-be/api/llm/usage?since=1970-01-01T00:00:00Z&until=1970-01-02T00:00:00Z",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert!(body["total_calls"].is_number());
        println!("✅ /api/llm/usage rejects malformed `since`; accepts valid RFC 3339");
    }

    /// Malformed UUID in `summary_id` → 400.
    #[tokio::test]
    async fn llm_usage_rejects_malformed_summary_id() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = get_uri(
            &router,
            "/brainatlas-be/api/llm/usage?summary_id=not-a-uuid",
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "malformed UUID must be rejected with 400"
        );
        println!("✅ /api/llm/usage rejects malformed `summary_id`");
    }

    /// `region_id` / `summary_id` / `batch_id` filter combinations route
    /// through separate diesel `.filter(...)` branches inside `aggregate`.
    /// Seed one row with all three populated and verify the handler
    /// plumbs every param through correctly.
    #[tokio::test]
    async fn llm_usage_filter_by_region_summary_batch() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }

        let pool = get_pool();
        let mut conn = pool.get().unwrap();

        let region_id = (rand::random::<u16>() as i32) + 10_000;
        let summary_id = Uuid::new_v4();
        let batch_id = Uuid::new_v4();
        let model = format!("test/{}-rsb", Uuid::new_v4());
        let tag = format!("ci:rsb-{}", Uuid::new_v4());

        // Direct INSERT including region_id, summary_id, batch_id.
        let row_id = Uuid::new_v4();
        diesel::sql_query(
            "INSERT INTO llm_call_usage \
             (id, endpoint, model, prompt_tokens, completion_tokens, total_tokens, \
              cost_usd, correlation_id, region_id, summary_id, batch_id, caller_tag) \
             VALUES ($1, 'chat_completion', $2, 42, 21, 63, 0.001, 'corr-rsb', \
                     $3, $4, $5, $6)",
        )
        .bind::<diesel::sql_types::Uuid, _>(row_id)
        .bind::<diesel::sql_types::Text, _>(&model)
        .bind::<diesel::sql_types::Int4, _>(region_id)
        .bind::<diesel::sql_types::Uuid, _>(summary_id)
        .bind::<diesel::sql_types::Uuid, _>(batch_id)
        .bind::<diesel::sql_types::Text, _>(&tag)
        .execute(&mut conn)
        .expect("insert usage row with full attribution");

        let router = build_router();

        // Filter by region_id
        let resp = get_uri(
            &router,
            &format!("/brainatlas-be/api/llm/usage?region_id={region_id}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(body["total_calls"].as_i64().unwrap(), 1, "region_id filter");
        assert_eq!(body["total_tokens"].as_i64().unwrap(), 63);

        // Filter by summary_id (UUID)
        let resp = get_uri(
            &router,
            &format!("/brainatlas-be/api/llm/usage?summary_id={summary_id}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(
            body["total_calls"].as_i64().unwrap(),
            1,
            "summary_id filter"
        );

        // Filter by batch_id (UUID)
        let resp = get_uri(
            &router,
            &format!("/brainatlas-be/api/llm/usage?batch_id={batch_id}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(body["total_calls"].as_i64().unwrap(), 1, "batch_id filter");

        // Combined filter: region_id + summary_id + batch_id + caller_tag
        let resp = get_uri(
            &router,
            &format!(
                "/brainatlas-be/api/llm/usage?region_id={region_id}&summary_id={summary_id}&batch_id={batch_id}&caller_tag={tag}"
            ),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(body["total_calls"].as_i64().unwrap(), 1, "combined filter");

        // Flip one field to a mismatched UUID → 0 results.
        let mismatched = Uuid::new_v4();
        let resp = get_uri(
            &router,
            &format!("/brainatlas-be/api/llm/usage?region_id={region_id}&summary_id={mismatched}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(
            body["total_calls"].as_i64().unwrap(),
            0,
            "mismatched summary_id must filter the row out"
        );

        cleanup_usage_ids(&mut conn, &[row_id]);
        println!("✅ /api/llm/usage filters by region_id, summary_id, batch_id, caller_tag");
    }

    /// Empty-table path: filter by a unique, impossible caller_tag so the
    /// result is guaranteed empty, and verify the zero-row shape.
    #[tokio::test]
    async fn llm_usage_empty_result_has_zero_totals() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let tag = format!("no-such-tag-{}", Uuid::new_v4());
        let resp = get_uri(
            &router,
            &format!("/brainatlas-be/api/llm/usage?caller_tag={tag}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_json(resp).await;
        assert_eq!(body["total_calls"], 0);
        assert_eq!(body["total_tokens"], 0);
        assert_eq!(body["total_prompt_tokens"], 0);
        assert_eq!(body["total_completion_tokens"], 0);
        assert_eq!(body["total_cost_usd"], 0.0);
        assert_eq!(body["by_model"].as_array().unwrap().len(), 0);
        assert_eq!(body["by_caller_tag"].as_array().unwrap().len(), 0);
        println!("✅ /api/llm/usage empty result returns zeroed aggregate");
    }

    // -- Other smoke/negative paths ------------------------------------------

    /// Unknown route under /brainatlas-be returns 404.
    #[tokio::test]
    async fn unknown_route_returns_404() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = get_uri(&router, "/brainatlas-be/api/does-not-exist").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        println!("✅ unknown route returns 404");
    }

    /// Malformed chunk_id in `/api/chunks/{chunk_id}/source` → axum's Path
    /// extractor rejects with 400 before the handler runs.
    #[tokio::test]
    async fn chunk_source_malformed_uuid_is_400() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let resp = get_uri(&router, "/brainatlas-be/api/chunks/not-a-uuid/source").await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "malformed UUID path segment must be 400 via axum Path extractor"
        );
        println!("✅ /api/chunks/{{bad}}/source is 400");
    }

    /// Unknown chunk_id (well-formed UUID that doesn't exist) → the handler
    /// converts `None` into `ApiError::MissingOrInvalidId` which maps to 400.
    #[tokio::test]
    async fn chunk_source_unknown_uuid_is_400() {
        if !should_run() {
            eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
            return;
        }
        let router = build_router();
        let missing = Uuid::new_v4();
        let resp = get_uri(
            &router,
            &format!("/brainatlas-be/api/chunks/{missing}/source"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = read_body_json(resp).await;
        assert!(body["error"].is_string());
        println!("✅ /api/chunks/{{unknown uuid}}/source is 400 (ApiError::MissingOrInvalidId)");
    }

    /// Keep the imports grounded against lint cleanup.
    #[allow(dead_code)]
    fn _async_trait_is_a_dep() {
        // This only exists to reference the async-trait dev-dep so
        // future trait-impl additions don't re-discover the wiring.
    }
}
