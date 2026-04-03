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
