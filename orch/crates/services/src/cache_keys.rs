use uuid::Uuid;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::future::Future;
use crate::CacheClient;

// ── TTL constants (seconds) ─────────────────────────────────────────────────

/// 10 minutes — for data that rarely changes (region mapping, chunk sources).
pub const TTL_LONG: u64 = 600;

/// 2 minutes — for data that changes on batch completion (summaries, config).
pub const TTL_MEDIUM: u64 = 120;

/// 15 seconds — for data that changes frequently (pipeline stats, batch/region status).
pub const TTL_SHORT: u64 = 15;

// ── Key builders ────────────────────────────────────────────────────────────

/// `orch:regions:all` — the full region_mapping list.
pub fn all_regions() -> String {
    "orch:regions:all".to_string()
}

/// `orch:pipeline:stats` — cross-region pipeline statistics.
pub fn pipeline_stats() -> String {
    "orch:pipeline:stats".to_string()
}

/// `orch:region:{id}:summaries` — summaries for one region.
pub fn region_summaries(id: Uuid) -> String {
    format!("orch:region:{}:summaries", id)
}

/// `orch:region:{id}:status` — pipeline status for one region.
pub fn region_status(id: Uuid) -> String {
    format!("orch:region:{}:status", id)
}

/// `orch:batch:{id}:status` — status of one batch.
pub fn batch_status(id: Uuid) -> String {
    format!("orch:batch:{}:status", id)
}

/// `orch:config:all` — full orch configuration.
pub fn config_all() -> String {
    "orch:config:all".to_string()
}

/// `orch:chunk:{id}:source` — chunk source resolution (immutable).
pub fn chunk_source(id: Uuid) -> String {
    format!("orch:chunk:{}:source", id)
}

/// `orch:batches:status:{status}` — batches by status (used by pipeline stats).
pub fn batches_by_status(status: &str) -> String {
    format!("orch:batches:status:{}", status)
}

// ── Invalidation patterns ───────────────────────────────────────────────────

/// `orch:region:{id}:*` — all cached data for a region.
pub fn region_pattern(id: Uuid) -> String {
    format!("orch:region:{}:*", id)
}

/// `orch:batches:status:*` — all per-status batch caches.
pub fn batches_status_pattern() -> String {
    "orch:batches:status:*".to_string()
}

/// `orch:search:{query}` — cached reverse search results for a given query.
pub fn search_results(query: &str) -> String {
    format!("orch:search:{}", query.to_lowercase())
}

/// `orch:search:*` — all cached search results (for invalidation).
pub fn search_pattern() -> String {
    "orch:search:*".to_string()
}

// ── Read-through cache helper ───────────────────────────────────────────────

/// Try the cache first; on miss, call `fetch_fn`, store the result, and return it.
///
/// All cache failures (get or set) are swallowed — the function always falls
/// through to `fetch_fn` if the cache is unavailable. This guarantees that a
/// Redis outage never causes a request to fail.
pub async fn cached_or_fetch<T, E, C, F, Fut>(
    cache: &C,
    key: &str,
    ttl_secs: u64,
    fetch_fn: F,
) -> Result<T, E>
where
    T: Serialize + DeserializeOwned,
    C: CacheClient,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    // Try cache hit
    if let Ok(Some(cached)) = cache.cache_get(key).await {
        if let Ok(value) = serde_json::from_str::<T>(&cached) {
            tracing::debug!(key, "cache hit");
            return Ok(value);
        }
        // Deserialization failed — treat as miss (stale/corrupt entry)
        tracing::warn!(key, "cache deserialization failed, treating as miss");
    }

    tracing::debug!(key, "cache miss");

    // Fetch from source
    let value = fetch_fn().await?;

    // Best-effort populate cache
    if let Ok(json) = serde_json::to_string(&value) {
        if let Err(e) = cache.cache_set(key, &json, ttl_secs).await {
            tracing::warn!(key, error = %e, "failed to populate cache");
        }
    }

    Ok(value)
}

/// Fire-and-forget invalidation of a single key. Logs on failure but never
/// propagates the error.
pub async fn invalidate<C: CacheClient>(cache: &C, key: &str) {
    if let Err(e) = cache.cache_del(key).await {
        tracing::warn!(key, error = %e, "cache invalidation failed");
    }
}

/// Fire-and-forget invalidation of keys matching a glob pattern.
pub async fn invalidate_pattern<C: CacheClient>(cache: &C, pattern: &str) {
    if let Err(e) = cache.cache_del_pattern(pattern).await {
        tracing::warn!(pattern, error = %e, "cache pattern invalidation failed");
    }
}
