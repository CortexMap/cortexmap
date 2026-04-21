// Integration tests for `orch/crates/infra/src/redis.rs` (Plan Task 2.3).
//
// Covers every method on `services::CacheClient`:
//   cache_get / cache_set (incl. TTL) / cache_del / cache_del_pattern / cache_stats
//
// All tests use unique key prefixes built from `uuid::Uuid::new_v4()` so they
// can share a Redis instance without interfering. Each test cleans up its own
// keys.
//
// To run:
//   docker compose -f docker-compose.test.yml up -d
//   RUN_INTEGRATION_TESTS=1 REDIS_URL=redis://127.0.0.1:6380 \
//     cargo test --package infra --test redis_integration -- --test-threads=1

use infra::OrchInfra;
use services::CacheClient;
use std::env;
use std::time::Duration;
use uuid::Uuid;

fn should_run() -> bool {
    env::var("RUN_INTEGRATION_TESTS").is_ok()
}

/// A prefix unique to this test run so we can safely `cache_del_pattern`
/// without colliding with concurrent work against the shared test Redis.
fn unique_prefix(tag: &str) -> String {
    format!("orch_test:{tag}:{}", Uuid::new_v4())
}

#[tokio::test]
async fn cache_set_and_cache_get_roundtrip_json() {
    if !should_run() {
        eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
        return;
    }

    let infra = OrchInfra::new();
    let key = unique_prefix("roundtrip");
    let value = serde_json::json!({
        "region_id": "11111111-2222-3333-4444-555555555555",
        "count": 42,
        "nested": {"ok": true, "tags": ["a", "b"]},
    })
    .to_string();

    infra
        .cache_set(&key, &value, 60)
        .await
        .expect("cache_set should succeed");

    let fetched = infra
        .cache_get(&key)
        .await
        .expect("cache_get should succeed");

    assert_eq!(
        fetched.as_deref(),
        Some(value.as_str()),
        "round-tripped value must equal what was set"
    );

    // And it must deserialise back to the same JSON shape.
    let parsed: serde_json::Value =
        serde_json::from_str(&fetched.unwrap()).expect("must be valid JSON");
    assert_eq!(parsed["count"], 42);
    assert_eq!(parsed["nested"]["ok"], true);

    // cleanup
    infra.cache_del(&key).await.ok();
}

#[tokio::test]
async fn cache_get_returns_none_for_missing_key() {
    if !should_run() {
        eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
        return;
    }

    let infra = OrchInfra::new();
    let key = unique_prefix("missing");

    let fetched = infra
        .cache_get(&key)
        .await
        .expect("cache_get must not error on a missing key");

    assert!(
        fetched.is_none(),
        "expected None for never-set key, got {fetched:?}"
    );
}

#[tokio::test]
async fn cache_set_with_ttl_expires() {
    if !should_run() {
        eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
        return;
    }

    let infra = OrchInfra::new();
    let key = unique_prefix("ttl");

    infra
        .cache_set(&key, "short-lived", 1)
        .await
        .expect("cache_set with ttl=1 should succeed");

    // Immediately visible.
    let before = infra.cache_get(&key).await.expect("get should succeed");
    assert_eq!(before.as_deref(), Some("short-lived"));

    // Wait 1.5s; Redis TTL granularity is seconds.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let after = infra.cache_get(&key).await.expect("get should succeed");
    assert!(
        after.is_none(),
        "expected key to have expired after 1.5s, still have {after:?}"
    );
}

#[tokio::test]
async fn cache_del_removes_key() {
    if !should_run() {
        eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
        return;
    }

    let infra = OrchInfra::new();
    let key = unique_prefix("del");

    infra.cache_set(&key, "bye", 60).await.expect("set");
    assert_eq!(infra.cache_get(&key).await.unwrap().as_deref(), Some("bye"));

    infra.cache_del(&key).await.expect("del");
    assert!(
        infra.cache_get(&key).await.unwrap().is_none(),
        "key should be gone after cache_del"
    );

    // Second del on a non-existent key must be idempotent.
    infra
        .cache_del(&key)
        .await
        .expect("idempotent del on missing key should succeed");
}

#[tokio::test]
async fn cache_del_pattern_matches_glob() {
    if !should_run() {
        eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
        return;
    }

    let infra = OrchInfra::new();
    // Two distinct namespaces, both test-unique.
    let ns_target = unique_prefix("ptn_target");
    let ns_other = unique_prefix("ptn_other");

    // Seed three "target" keys and one "other" key that must survive.
    let ka = format!("{ns_target}:a");
    let kb = format!("{ns_target}:b");
    let kc = format!("{ns_target}:c");
    let other = format!("{ns_other}:survivor");

    for (k, v) in [(&ka, "a"), (&kb, "b"), (&kc, "c"), (&other, "keep")] {
        infra.cache_set(k, v, 60).await.expect("seed");
    }

    // Delete `<ns_target>:*`.
    let deleted = infra
        .cache_del_pattern(&format!("{ns_target}:*"))
        .await
        .expect("cache_del_pattern");

    assert!(
        deleted >= 3,
        "expected to delete at least 3 keys, got {deleted}"
    );
    assert!(infra.cache_get(&ka).await.unwrap().is_none(), "ka gone");
    assert!(infra.cache_get(&kb).await.unwrap().is_none(), "kb gone");
    assert!(infra.cache_get(&kc).await.unwrap().is_none(), "kc gone");

    // The unrelated key must survive.
    let survivor = infra.cache_get(&other).await.unwrap();
    assert_eq!(
        survivor.as_deref(),
        Some("keep"),
        "unrelated namespace must not be touched"
    );

    // cleanup
    infra.cache_del(&other).await.ok();

    // Deleting a pattern that matches nothing returns 0.
    let zero = infra
        .cache_del_pattern(&format!("orch_test:never_seeded:{}:*", Uuid::new_v4()))
        .await
        .expect("cache_del_pattern on empty must not error");
    assert_eq!(zero, 0);
}

#[tokio::test]
async fn cache_stats_when_connected() {
    if !should_run() {
        eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
        return;
    }

    let infra = OrchInfra::new();

    // Seed at least one key so `total_keys >= 1` is deterministic.
    let key = unique_prefix("stats_seed");
    infra.cache_set(&key, "present", 60).await.expect("seed");

    let stats = infra.cache_stats().await.expect("cache_stats must not error");

    assert!(stats.connected, "stats.connected must be true when Redis is reachable");
    assert!(stats.error.is_none(), "stats.error must be None when connected, got {:?}", stats.error);
    assert!(
        stats.total_keys >= 1,
        "total_keys ({}) should be >= 1 after seeding",
        stats.total_keys
    );
    // `keys_by_prefix` is seeded with a fixed list in the production code.
    assert!(
        !stats.keys_by_prefix.is_empty(),
        "keys_by_prefix must list the canonical prefixes"
    );
    // `server_version` / `used_memory_human` are best-effort but must be
    // populated from `INFO` output in the connected path.
    assert!(
        !stats.server_version.is_empty(),
        "server_version should be set when connected"
    );

    infra.cache_del(&key).await.ok();
}

/// Connection-failure branch of `cache_stats` — must degrade to a
/// `connected: false` snapshot rather than propagate an error, so the dev
/// dashboard can render.
///
/// This variant is `#[ignore]` by default: `OrchRedis::conn()` memoises the
/// connection via a `OnceCell`, and `redis::aio::ConnectionManager::new`
/// (v0.27) performs retries with unbounded per-attempt timeouts which can
/// take 30+ seconds against a refused port, making the test too slow for
/// default CI. It still exercises the graceful-degradation branch at
/// `orch/crates/infra/src/redis.rs:85-106` when run manually:
///
///   RUN_INTEGRATION_TESTS=1 REDIS_URL=redis://127.0.0.1:1 \
///     cargo test --package infra --test redis_integration \
///       cache_stats_when_redis_down -- --ignored --test-threads=1
///
/// Note: do NOT set `REDIS_URL` to the real test Redis when running this —
/// point it at an unreachable endpoint for the whole binary invocation.
#[tokio::test]
#[ignore = "slow: redis ConnectionManager retries for ~30s against a refused port; run manually with --ignored"]
async fn cache_stats_when_redis_down() {
    if !should_run() {
        eprintln!("RUN_INTEGRATION_TESTS not set, skipping");
        return;
    }

    // This test must be invoked with REDIS_URL pointing at an unreachable
    // endpoint. It does NOT mutate the env to avoid poisoning other tests
    // sharing the process (`OrchRedis::conn()` memoises via OnceCell).
    let configured = env::var("REDIS_URL").unwrap_or_default();
    if !configured.contains(":1") && !configured.contains(":65534") {
        panic!(
            "cache_stats_when_redis_down requires REDIS_URL to point at an \
             unreachable endpoint (e.g. redis://127.0.0.1:1); got {configured:?}"
        );
    }

    let infra = OrchInfra::new();
    let stats = infra
        .cache_stats()
        .await
        .expect("cache_stats must return Ok even when Redis is down");

    assert!(
        !stats.connected,
        "stats.connected should be false when Redis is unreachable, got {stats:?}"
    );
    assert!(
        stats.error.is_some(),
        "stats.error should be populated with the underlying error"
    );
    assert_eq!(stats.total_keys, 0, "total_keys must be 0 in the down path");
    assert!(
        stats.keys_by_prefix.is_empty(),
        "keys_by_prefix must be empty in the down path"
    );
}
