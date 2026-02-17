use crate::FetchError;
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{HttpInfra, InfraContext, TaskQueueInfra};

/// Enqueue all PMC IDs from a query into the task queue
///
/// This function:
/// 1. Queries NCBI ESearch to get PMC IDs for the query
/// 2. For each PMC ID, creates a fetch task in the database
/// 3. For each task, creates three component records (summary, abstract, pdf)
/// 4. Returns the list of (pmc_id, task_id) tuples
pub async fn enqueue_query<I>(
    blueprint: &Blueprint,
    ctx: InfraContext<I>,
) -> Result<Vec<(String, i64)>, FetchError>
where
    I: HttpInfra + TaskQueueInfra + Send + Sync + 'static,
{
    // Query NCBI ESearch to get PMC IDs using URL from blueprint
    let search_url = blueprint.fetcher.esearch_url
        .replace("{query}", &blueprint.fetcher.query)
        .replace("{pageSize}", &blueprint.fetcher.page_size.to_string());

    tracing::info!("Fetching PMC IDs from: {}", search_url);
    let search_resp = ctx.infra.get(&search_url).await?;
    let search_result: serde_json::Value = serde_json::from_slice(&search_resp.bytes().await?)?;

    // Extract ID list from response
    let id_list = search_result
        .get("esearchresult")
        .and_then(|r| r.get("idlist"))
        .and_then(|l| l.as_array())
        .ok_or_else(|| FetchError::NotFound("No idlist in ESearch response".to_string()))?;

    if id_list.is_empty() {
        tracing::info!("No PMC IDs found for query: {}", blueprint.fetcher.query);
        return Ok(vec![]);
    }

    let pmc_ids: Vec<String> = id_list
        .iter()
        .filter_map(|v| v.as_str())
        .map(|s| {
            // Ensure PMC prefix
            if s.starts_with("PMC") {
                s.to_string()
            } else {
                format!("PMC{}", s)
            }
        })
        .collect();

    tracing::info!("Found {} PMC IDs, enqueueing tasks...", pmc_ids.len());

    let mut enqueued = Vec::new();
    let max_attempts = blueprint.fetcher.max_retry_attempts as i32;

    for pmc_id in pmc_ids {
        match ctx
            .infra
            .enqueue_task(pmc_id.clone(), blueprint.fetcher.query.clone(), max_attempts)
            .await
        {
            Ok(task) => {
                tracing::info!(
                    "Enqueued task {} for PMC {} with {} max attempts",
                    task.id,
                    pmc_id,
                    max_attempts
                );
                enqueued.push((pmc_id, task.id));
            }
            Err(e) => {
                tracing::warn!("Failed to enqueue task for PMC {}: {}", pmc_id, e);
                // Continue with other PMC IDs even if one fails
            }
        }
    }

    tracing::info!(
        "Successfully enqueued {}/{} tasks",
        enqueued.len(),
        id_list.len()
    );

    Ok(enqueued)
}

#[cfg(test)]
mod tests {
    use cortexmap_core::blueprint::connections::Fetcher;

    #[test]
    fn test_esearch_url_replacement() {
        let fetcher = Fetcher::default();
        let url = fetcher.esearch_url
            .replace("{query}", "neuroscience")
            .replace("{pageSize}", "10");
        
        assert!(url.contains("neuroscience"));
        assert!(url.contains("retmax=10"));
    }
}
