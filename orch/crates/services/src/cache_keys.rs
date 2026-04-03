use crate::CacheClient;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use uuid::Uuid;

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
    if let Ok(json) = serde_json::to_string(&value)
        && let Err(e) = cache.cache_set(key, &json, ttl_secs).await
    {
        tracing::warn!(key, error = %e, "failed to populate cache");
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::io;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct CachedValue {
        value: String,
    }

    #[derive(Default)]
    struct FakeCache {
        entries: Mutex<HashMap<String, String>>,
        set_calls: Mutex<Vec<(String, String, u64)>>,
        del_calls: Mutex<Vec<String>>,
        del_pattern_calls: Mutex<Vec<String>>,
        fail_get: AtomicBool,
        fail_set: AtomicBool,
        fail_del: AtomicBool,
        fail_del_pattern: AtomicBool,
    }

    #[async_trait::async_trait]
    impl CacheClient for FakeCache {
        type Error = io::Error;

        async fn cache_get(&self, key: &str) -> Result<Option<String>, Self::Error> {
            if self.fail_get.load(Ordering::SeqCst) {
                return Err(io::Error::other("cache get failed"));
            }

            Ok(self.entries.lock().unwrap().get(key).cloned())
        }

        async fn cache_set(
            &self,
            key: &str,
            value: &str,
            ttl_secs: u64,
        ) -> Result<(), Self::Error> {
            if self.fail_set.load(Ordering::SeqCst) {
                return Err(io::Error::other("cache set failed"));
            }

            self.entries
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            self.set_calls
                .lock()
                .unwrap()
                .push((key.to_string(), value.to_string(), ttl_secs));
            Ok(())
        }

        async fn cache_del(&self, key: &str) -> Result<(), Self::Error> {
            if self.fail_del.load(Ordering::SeqCst) {
                return Err(io::Error::other("cache delete failed"));
            }

            self.del_calls.lock().unwrap().push(key.to_string());
            self.entries.lock().unwrap().remove(key);
            Ok(())
        }

        async fn cache_del_pattern(&self, pattern: &str) -> Result<u64, Self::Error> {
            if self.fail_del_pattern.load(Ordering::SeqCst) {
                return Err(io::Error::other("cache delete pattern failed"));
            }

            self.del_pattern_calls
                .lock()
                .unwrap()
                .push(pattern.to_string());
            Ok(0)
        }
    }

    #[test]
    fn test_key_builders_and_patterns_are_stable() {
        let id = Uuid::parse_str("a8f0c6bb-06a3-4b6f-8d60-a3d3066f1f70").unwrap();

        assert_eq!(all_regions(), "orch:regions:all");
        assert_eq!(pipeline_stats(), "orch:pipeline:stats");
        assert_eq!(region_summaries(id), format!("orch:region:{id}:summaries"));
        assert_eq!(region_status(id), format!("orch:region:{id}:status"));
        assert_eq!(batch_status(id), format!("orch:batch:{id}:status"));
        assert_eq!(config_all(), "orch:config:all");
        assert_eq!(chunk_source(id), format!("orch:chunk:{id}:source"));
        assert_eq!(batches_by_status("ready"), "orch:batches:status:ready");
        assert_eq!(region_pattern(id), format!("orch:region:{id}:*"));
        assert_eq!(batches_status_pattern(), "orch:batches:status:*");
        assert_eq!(search_results("HippoCampus"), "orch:search:hippocampus");
        assert_eq!(search_pattern(), "orch:search:*");
    }

    #[tokio::test]
    async fn test_cached_or_fetch_returns_cached_value_without_invoking_fetch() {
        let cache = FakeCache::default();
        let key = "orch:test:cached";
        let expected = CachedValue {
            value: "cached".to_string(),
        };
        cache
            .entries
            .lock()
            .unwrap()
            .insert(key.to_string(), serde_json::to_string(&expected).unwrap());

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fetch_count_clone = Arc::clone(&fetch_count);
        let result = cached_or_fetch(&cache, key, TTL_SHORT, move || {
            let fetch_count = Arc::clone(&fetch_count_clone);
            async move {
                fetch_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, io::Error>(CachedValue {
                    value: "fresh".to_string(),
                })
            }
        })
        .await
        .unwrap();

        assert_eq!(result, expected);
        assert_eq!(fetch_count.load(Ordering::SeqCst), 0);
        assert!(cache.set_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_cached_or_fetch_populates_cache_on_miss() {
        let cache = FakeCache::default();
        let key = "orch:test:miss";

        let result = cached_or_fetch(&cache, key, TTL_MEDIUM, || async {
            Ok::<_, io::Error>(CachedValue {
                value: "fresh".to_string(),
            })
        })
        .await
        .unwrap();

        assert_eq!(result.value, "fresh");

        let set_calls = cache.set_calls.lock().unwrap();
        assert_eq!(set_calls.len(), 1);
        assert_eq!(set_calls[0].0, key);
        assert_eq!(set_calls[0].2, TTL_MEDIUM);
        assert_eq!(
            cache.entries.lock().unwrap().get(key).unwrap(),
            &serde_json::to_string(&result).unwrap()
        );
    }

    #[tokio::test]
    async fn test_cached_or_fetch_refreshes_corrupt_entries() {
        let cache = FakeCache::default();
        let key = "orch:test:corrupt";
        cache
            .entries
            .lock()
            .unwrap()
            .insert(key.to_string(), "not valid json".to_string());

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fetch_count_clone = Arc::clone(&fetch_count);
        let result = cached_or_fetch(&cache, key, TTL_LONG, move || {
            let fetch_count = Arc::clone(&fetch_count_clone);
            async move {
                fetch_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, io::Error>(CachedValue {
                    value: "recovered".to_string(),
                })
            }
        })
        .await
        .unwrap();

        assert_eq!(result.value, "recovered");
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
        assert_eq!(cache.set_calls.lock().unwrap()[0].2, TTL_LONG);
    }

    #[tokio::test]
    async fn test_cached_or_fetch_swallows_cache_get_and_set_errors() {
        let cache = FakeCache::default();
        cache.fail_get.store(true, Ordering::SeqCst);
        cache.fail_set.store(true, Ordering::SeqCst);

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fetch_count_clone = Arc::clone(&fetch_count);
        let result = cached_or_fetch(&cache, "orch:test:errors", TTL_SHORT, move || {
            let fetch_count = Arc::clone(&fetch_count_clone);
            async move {
                fetch_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, io::Error>(CachedValue {
                    value: "fallback".to_string(),
                })
            }
        })
        .await
        .unwrap();

        assert_eq!(result.value, "fallback");
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
        assert!(cache.set_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_invalidate_deletes_single_key() {
        let cache = FakeCache::default();
        invalidate(&cache, "orch:test:delete").await;

        assert_eq!(
            cache.del_calls.lock().unwrap().clone(),
            vec!["orch:test:delete".to_string()]
        );
    }

    #[tokio::test]
    async fn test_invalidation_helpers_swallow_delete_failures() {
        let cache = FakeCache::default();
        cache.fail_del.store(true, Ordering::SeqCst);
        cache.fail_del_pattern.store(true, Ordering::SeqCst);

        invalidate(&cache, "orch:test:delete").await;
        invalidate_pattern(&cache, "orch:test:*").await;
    }

    #[tokio::test]
    async fn test_invalidate_pattern_deletes_matching_keys() {
        let cache = FakeCache::default();
        invalidate_pattern(&cache, "orch:search:*").await;

        assert_eq!(
            cache.del_pattern_calls.lock().unwrap().clone(),
            vec!["orch:search:*".to_string()]
        );
    }
}
