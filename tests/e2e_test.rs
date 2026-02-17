// End-to-end integration test that tests the entire pipeline
// This test requires all services to be running (fetcher, brainatlas, orch)

use serde_json::{json, Value};
use uuid::Uuid;
use tokio::time::{sleep, Duration};

#[tokio::test]
#[ignore] // Run with --ignored flag when services are running
async fn test_complete_pipeline_flow() {
    let client = reqwest::Client::new();
    
    // Assume services are running on localhost with test ports
    let fetcher_url = "http://localhost:8080";
    let brainatlas_url = "http://localhost:8081";
    let orch_url = "http://localhost:8082";
    
    println!("=== Step 1: Health Checks ===");
    
    // Check fetcher health
    let response = client.get(format!("{}/fetcher-be/health", fetcher_url))
        .send()
        .await
        .expect("Fetcher service not running");
    assert_eq!(response.status(), 200);
    println!("✅ Fetcher is healthy");
    
    // Check brainatlas health
    let response = client.get(format!("{}/brainatlas-be/health", brainatlas_url))
        .send()
        .await
        .expect("BrainAtlas service not running");
    assert_eq!(response.status(), 200);
    println!("✅ BrainAtlas is healthy");
    
    // Check orch health
    let response = client.get(format!("{}/orch/health", orch_url))
        .send()
        .await
        .expect("Orch service not running");
    assert_eq!(response.status(), 200);
    println!("✅ Orch is healthy");
    
    println!("\n=== Step 2: Get All Regions ===");
    
    let response = client.get(format!("{}/orch/api/regions", orch_url))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    let regions: Vec<Value> = response.json().await.unwrap();
    
    assert!(!regions.is_empty(), "No regions found in database");
    println!("✅ Found {} regions", regions.len());
    
    // Pick the first region for testing
    let test_region = &regions[0];
    let region_id = test_region["id"].as_str().unwrap();
    let region_name = test_region["name"].as_str().unwrap_or("Unknown");
    
    println!("Testing with region: {} ({})", region_name, region_id);
    
    println!("\n=== Step 3: Search Region (Trigger Pipeline) ===");
    
    let response = client.post(format!("{}/orch/api/regions/{}/search", orch_url, region_id))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    let search_response: Value = response.json().await.unwrap();
    
    println!("Search response: {}", serde_json::to_string_pretty(&search_response).unwrap());
    
    // If summaries already exist, skip the rest
    if let Some(summaries) = search_response.get("summaries") {
        if summaries.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
            println!("✅ Summaries already exist for this region");
            return;
        }
    }
    
    println!("\n=== Step 4: Monitor Pipeline Progress ===");
    
    // Poll region status until completed or failed
    let max_retries = 60; // 5 minutes with 5-second intervals
    let mut retry_count = 0;
    
    loop {
        sleep(Duration::from_secs(5)).await;
        retry_count += 1;
        
        let response = client.get(format!("{}/orch/api/regions/{}/status", orch_url, region_id))
            .send()
            .await
            .unwrap();
        
        assert_eq!(response.status(), 200);
        let status: Value = response.json().await.unwrap();
        
        let current_status = status["status"].as_str().unwrap_or("Unknown");
        println!("  Status: {} (retry {}/{})", current_status, retry_count, max_retries);
        
        match current_status {
            "Completed" => {
                println!("✅ Pipeline completed successfully!");
                break;
            }
            "Failed" => {
                let error = status["last_error"].as_str().unwrap_or("Unknown error");
                panic!("❌ Pipeline failed: {}", error);
            }
            _ => {
                if retry_count >= max_retries {
                    panic!("❌ Pipeline did not complete within timeout");
                }
            }
        }
    }
    
    println!("\n=== Step 5: Verify Summaries Were Created ===");
    
    let response = client.post(format!("{}/orch/api/regions/{}/search", orch_url, region_id))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    let final_response: Value = response.json().await.unwrap();
    
    let summaries = final_response["summaries"].as_array()
        .expect("Should have summaries array");
    
    assert!(!summaries.is_empty(), "No summaries created");
    
    println!("✅ Found {} summaries", summaries.len());
    println!("Sample summary: {}", summaries[0]["summary"].as_str().unwrap_or("N/A").chars().take(100).collect::<String>());
    
    println!("\n=== Step 6: Verify Pipeline Stats ===");
    
    let response = client.get(format!("{}/orch/api/pipeline/stats", orch_url))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    let stats: Value = response.json().await.unwrap();
    
    println!("Pipeline stats:");
    println!("  Total regions: {}", stats["total_regions"]);
    println!("  Completed: {}", stats["completed"]);
    println!("  Processing: {}", stats["processing"]);
    println!("  Failed: {}", stats["failed"]);
    
    assert!(stats["completed"].as_i64().unwrap() > 0, "No completed batches");
    
    println!("\n🎉 End-to-end test passed!");
}

#[tokio::test]
#[ignore]
async fn test_invalidate_region_flow() {
    let client = reqwest::Client::new();
    let orch_url = "http://localhost:8082";
    
    println!("=== Testing Invalidate Region Flow ===");
    
    // Get a region
    let response = client.get(format!("{}/orch/api/regions", orch_url))
        .send()
        .await
        .unwrap();
    
    let regions: Vec<Value> = response.json().await.unwrap();
    assert!(!regions.is_empty());
    
    let region_id = regions[0]["id"].as_str().unwrap();
    
    // Invalidate the region
    let response = client.post(format!("{}/orch/api/regions/{}/invalidate", orch_url, region_id))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    println!("✅ Region invalidated successfully");
    
    // Check status
    let response = client.get(format!("{}/orch/api/regions/{}/status", orch_url, region_id))
        .send()
        .await
        .unwrap();
    
    let status: Value = response.json().await.unwrap();
    println!("Status after invalidation: {}", status["status"]);
}

#[tokio::test]
#[ignore]
async fn test_config_update_flow() {
    let client = reqwest::Client::new();
    let orch_url = "http://localhost:8082";
    
    println!("=== Testing Config Management ===");
    
    // Get current config
    let response = client.get(format!("{}/orch/api/config", orch_url))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    let config: Value = response.json().await.unwrap();
    println!("Current config: {}", serde_json::to_string_pretty(&config).unwrap());
    
    // Update config
    let update_request = json!({
        "query_generation_limit": 10
    });
    
    let response = client.patch(format!("{}/orch/api/config", orch_url))
        .json(&update_request)
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    println!("✅ Config updated successfully");
    
    // Verify update
    let response = client.get(format!("{}/orch/api/config", orch_url))
        .send()
        .await
        .unwrap();
    
    let updated_config: Value = response.json().await.unwrap();
    
    // Find query_generation_limit in config array
    let limit_entry = updated_config.as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["key"] == "query_generation_limit");
    
    if let Some(entry) = limit_entry {
        assert_eq!(entry["value"], "10");
        println!("✅ Config value verified");
    }
}
