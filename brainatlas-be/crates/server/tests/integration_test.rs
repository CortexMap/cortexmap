// Integration tests for BrainAtlas
// These tests require PostgreSQL and MinIO (via docker-compose)
//
// To run: cargo test --package server --test integration_test -- --test-threads=1
//
// Prerequisites:
// 1. docker-compose -f ../../docker-compose.test.yml up -d
// 2. diesel migration run --database-url postgresql://test_user:test_password@localhost:5433/test_db

use uuid::Uuid;
use std::env;

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
    env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://test_user:test_password@localhost:5433/test_db".to_string())
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
    #[ignore] // Requires running test infrastructure
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
    #[ignore] // Requires running test infrastructure
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
    #[ignore] // Requires running test infrastructure
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
    #[ignore] // Requires running test infrastructure
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
             ON CONFLICT (id) DO NOTHING"
        )
        .bind::<diesel::sql_types::Uuid, _>(test_uuid)
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>("Test Region")
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("TR"))
        .execute(&mut conn);
        
        assert!(insert_result.is_ok());
        
        // Query it back
        let query_result = diesel::sql_query(
            "SELECT id, region_id, name FROM region_mapping WHERE id = $1"
        )
        .bind::<diesel::sql_types::Uuid, _>(test_uuid)
        .get_result::<RegionMappingResult>(&mut conn);
        
        assert!(query_result.is_ok());
        let result = query_result.unwrap();
        assert_eq!(result.id, test_uuid);
        assert_eq!(result.region_id, test_region_id);
        assert_eq!(result.name, "Test Region");
        
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
    #[ignore] // Requires running test infrastructure
    async fn test_s3_connection() {
        let (endpoint, access_key, secret_key, _bucket) = get_test_s3_config();
        
        // Create S3 client
        use aws_sdk_s3::config::{Credentials, Region};
        use aws_sdk_s3::Client;
        
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
    #[ignore] // Requires running test infrastructure
    async fn test_s3_create_bucket_and_upload() {
        let (endpoint, access_key, secret_key, bucket) = get_test_s3_config();
        
        use aws_sdk_s3::config::{Credentials, Region};
        use aws_sdk_s3::Client;
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
        let _ = client
            .create_bucket()
            .bucket(&bucket)
            .send()
            .await;
        
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
        
        assert!(download_result.is_ok(), "Should be able to download from S3");
        
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
    #[ignore] // Requires running test infrastructure
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
             ON CONFLICT (id) DO NOTHING"
        )
        .bind::<diesel::sql_types::Uuid, _>(test_uuid)
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>("Test Empty Region")
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("TER"))
        .execute(&mut conn)
        .expect("Failed to insert test region");
        
        // Query summaries (should be empty)
        let count_result = diesel::sql_query(
            "SELECT COUNT(*) as count FROM region_summary WHERE region_id = $1"
        )
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .get_result::<CountResult>(&mut conn)
        .expect("Failed to count summaries");
        
        assert_eq!(count_result.count, 0, "Should have no summaries for new region");
        
        // Cleanup
        diesel::sql_query("DELETE FROM region_mapping WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(test_uuid)
            .execute(&mut conn)
            .ok();
        
        println!("✅ Search returns empty for region without summaries");
    }

    #[tokio::test]
    #[ignore] // Requires running test infrastructure
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
        let test_summary = "This is a test summary about the hippocampus and its role in memory formation.";
        
        // Insert test region
        diesel::sql_query(
            "INSERT INTO region_mapping (id, region_id, name, acronym) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO NOTHING"
        )
        .bind::<diesel::sql_types::Uuid, _>(test_uuid)
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>("Test Region With Summary")
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(Some("TRWS"))
        .execute(&mut conn)
        .expect("Failed to insert test region");
        
        // Insert summary
        diesel::sql_query(
            "INSERT INTO region_summary (id, region_id, name, summary, created_at, content_hash)
             VALUES ($1, $2, $3, $4, NOW(), $5)"
        )
        .bind::<diesel::sql_types::Uuid, _>(Uuid::new_v4())
        .bind::<diesel::sql_types::Int4, _>(test_region_id)
        .bind::<diesel::sql_types::Text, _>("Test Region With Summary")
        .bind::<diesel::sql_types::Text, _>(test_summary)
        .bind::<diesel::sql_types::Text, _>("test_hash_123")
        .execute(&mut conn)
        .expect("Failed to insert summary");
        
        // Query summaries
        let summaries: Vec<String> = diesel::sql_query(
            "SELECT summary FROM region_summary WHERE region_id = $1"
        )
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
    use diesel::prelude::*;
    use diesel::r2d2::{self, ConnectionManager};
    use aws_sdk_s3::config::{Credentials, Region};
    use aws_sdk_s3::Client;
    use aws_sdk_s3::primitives::ByteStream;

    #[tokio::test]
    #[ignore] // Requires running test infrastructure
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
             ON CONFLICT (id) DO NOTHING"
        )
        .bind::<diesel::sql_types::Uuid, _>(region_uuid)
        .bind::<diesel::sql_types::Int4, _>(region_id)
        .bind::<diesel::sql_types::Text, _>("Hippocampus")
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
        let generated_summary = format!(
            "The hippocampus is a critical brain region for memory formation and consolidation. \
             Research demonstrates its role in neuroplasticity and learning processes, \
             particularly during sleep when memory consolidation occurs through neural replay."
        );
        
        println!("✅ Step 3: Summary generated (simulated)");
        
        // Step 4: Store summary in database
        diesel::sql_query(
            "INSERT INTO region_summary (id, region_id, name, summary, created_at, content_hash)
             VALUES ($1, $2, $3, $4, NOW(), $5)"
        )
        .bind::<diesel::sql_types::Uuid, _>(Uuid::new_v4())
        .bind::<diesel::sql_types::Int4, _>(region_id)
        .bind::<diesel::sql_types::Text, _>("Hippocampus")
        .bind::<diesel::sql_types::Text, _>(generated_summary.clone())
        .bind::<diesel::sql_types::Text, _>("test_workflow_hash")
        .execute(&mut conn)
        .expect("Failed to insert summary");
        
        println!("✅ Step 4: Summary stored in database");
        
        // Step 5: Retrieve and verify summary
        let retrieved_summaries: Vec<String> = diesel::sql_query(
            "SELECT summary FROM region_summary WHERE region_id = $1"
        )
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
        
        let _ = s3_client.delete_object().bucket(&s3_bucket).key(&paper1_key).send().await;
        let _ = s3_client.delete_object().bucket(&s3_bucket).key(&paper2_key).send().await;
        
        println!("✅ Complete workflow test passed!");
    }
}
