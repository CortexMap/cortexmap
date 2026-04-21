//! Integration tests for the diesel-backed `EvalsPostgresql` adapter.
//!
//! All tests are gated on `RUN_INTEGRATION_TESTS=1`. They require a
//! Postgres reachable at `TEST_DATABASE_URL` (defaults to the
//! `docker-compose.test.yml` service on `localhost:5433`) with the
//! evals-be + brainatlas-be migrations already applied.
//!
//! Each test:
//!   * generates its own random UUIDs / `region_id`s (so tests can be
//!     run in parallel, even though CI pins `--test-threads=1`),
//!   * seeds its own `region_mapping` / `region_summary` rows (which
//!     `eval_scores` / `eval_runs` / `brain_region_embeddings` FK into),
//!   * explicitly cleans up its own rows at the end via a dedicated
//!     cleanup helper.
//!
//! Run locally:
//!   docker compose -f docker-compose.test.yml up -d
//!   RUN_INTEGRATION_TESTS=1 \
//!   TEST_DATABASE_URL=postgresql://test_user:test_password@localhost:5433/test_db \
//!   cargo test -p evals-infra --test pg_integration -- --test-threads=1

use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use domain::{EvalRunStatus, NewEvalScore};
use evals_infra::EvalsPostgresql;
use rand::Rng;
use services::ChunkRow;
use std::collections::HashMap;
use uuid::Uuid;

// ---------- Test helpers ----------

fn integration_enabled() -> bool {
    std::env::var("RUN_INTEGRATION_TESTS").is_ok()
}

fn get_test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://test_user:test_password@localhost:5433/test_db".to_string()
        })
}

/// Build a one-shot r2d2 pool for direct setup/seeding/cleanup. We
/// deliberately keep this separate from the `EvalsPostgresql`'s
/// deadpool so tests can inspect state the trait impl can't reach.
fn raw_pool() -> r2d2::Pool<ConnectionManager<PgConnection>> {
    let manager = ConnectionManager::<PgConnection>::new(get_test_db_url());
    r2d2::Pool::builder()
        .max_size(1)
        .build(manager)
        .expect("build raw test pool")
}

/// Pick a large-but-still-i32-safe integer that is unlikely to collide
/// with anything seeded by other tests or fixtures. `rand::random::<u16>()`
/// keeps us under `i32::MAX` and `+ 10_000` keeps us out of any
/// low-valued fixture territory.
fn unique_region_id() -> i32 {
    let mut rng = rand::thread_rng();
    rng.r#gen::<u16>() as i32 + 10_000
}

/// Seed the FK-required upstream rows and return
/// `(summary_id, region_id)`. Cleaning up the summary row cascades
/// into `eval_scores`, `eval_runs`, and `brain_region_embeddings`.
fn seed_region_and_summary(
    conn: &mut PgConnection,
    summary_text: Option<&str>,
    name: &str,
) -> (Uuid, i32) {
    let region_id = unique_region_id();
    let summary_id = Uuid::new_v4();
    let batch_id = Uuid::new_v4();

    diesel::sql_query(
        "INSERT INTO region_mapping (region_id, name) VALUES ($1, $2)",
    )
    .bind::<diesel::sql_types::Integer, _>(region_id)
    .bind::<diesel::sql_types::Text, _>(name)
    .execute(conn)
    .expect("seed region_mapping");

    diesel::sql_query(
        "INSERT INTO region_summary (id, region_id, name, summary, batch_id, is_active)
         VALUES ($1, $2, $3, $4, $5, TRUE)",
    )
    .bind::<diesel::sql_types::Uuid, _>(summary_id)
    .bind::<diesel::sql_types::Integer, _>(region_id)
    .bind::<diesel::sql_types::Text, _>(name)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(summary_text)
    .bind::<diesel::sql_types::Uuid, _>(batch_id)
    .execute(conn)
    .expect("seed region_summary");

    (summary_id, region_id)
}

/// Cleanup helper — deletes everything seeded for this `(summary_id, region_id)`.
/// `ON DELETE CASCADE` on `eval_scores` / `eval_runs` / `brain_region_embeddings`
/// means deleting `region_summary` is enough. `eval_run_state` has no FK and
/// must be cleaned up explicitly by `summary_id`.
fn cleanup(conn: &mut PgConnection, summary_id: Uuid, region_id: i32) {
    diesel::sql_query("DELETE FROM eval_run_state WHERE summary_id = $1")
        .bind::<diesel::sql_types::Uuid, _>(summary_id)
        .execute(conn)
        .ok();
    diesel::sql_query("DELETE FROM region_summary WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(summary_id)
        .execute(conn)
        .ok();
    diesel::sql_query("DELETE FROM region_mapping WHERE region_id = $1")
        .bind::<diesel::sql_types::Integer, _>(region_id)
        .execute(conn)
        .ok();
}

/// Unique tag used in `eval_version` / `metric` so cross-test state can't
/// contaminate aggregates.
fn unique_tag(prefix: &str) -> String {
    let mut rng = rand::thread_rng();
    format!("{prefix}_{:x}", rng.r#gen::<u32>())
}

// ---------- Tests ----------

/// 1. Insert a `NewEvalScore`, look it up by `(hash, metric, eval_version)`,
/// assert every field round-trips.
#[tokio::test]
async fn lookup_score_by_hash_and_insert_score_roundtrip() {
    if !integration_enabled() {
        eprintln!("skipping: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let url = get_test_db_url();
    let pool = raw_pool();
    let conn = &mut pool.get().unwrap();
    let (summary_id, region_id) =
        seed_region_and_summary(conn, Some("body"), "lookup_score_test_region");

    let pg = EvalsPostgresql::new();
    let hash = unique_tag("hash");
    let metric = "rubric_relevance".to_string();
    let eval_version = unique_tag("v");

    let new = NewEvalScore {
        summary_id,
        summary_hash: hash.clone(),
        metric: metric.clone(),
        score: 0.75,
        judge_model: Some("openai/gpt-4o-mini".to_string()),
        details: Some(serde_json::json!({"reason": "ok"})),
        eval_version: eval_version.clone(),
    };

    let inserted = pg
        .insert_score(&url, new.clone())
        .await
        .expect("insert_score");
    assert_eq!(inserted.summary_id, summary_id);
    assert_eq!(inserted.summary_hash, hash);
    assert_eq!(inserted.metric, metric);
    assert!((inserted.score - 0.75).abs() < 1e-6);
    assert_eq!(inserted.judge_model.as_deref(), Some("openai/gpt-4o-mini"));
    assert_eq!(inserted.eval_version, eval_version);
    assert_eq!(
        inserted.details.as_ref().and_then(|v| v.get("reason")),
        Some(&serde_json::Value::String("ok".into()))
    );

    let looked = pg
        .lookup_score_by_hash(&url, &hash, &metric, &eval_version)
        .await
        .expect("lookup_score_by_hash")
        .expect("row should exist");
    assert_eq!(looked.id, inserted.id);
    assert_eq!(looked.summary_hash, hash);
    assert_eq!(looked.metric, metric);
    assert_eq!(looked.eval_version, eval_version);

    // Unknown hash returns None (not an error).
    let missing = pg
        .lookup_score_by_hash(&url, "nope", &metric, &eval_version)
        .await
        .expect("lookup_score_by_hash missing");
    assert!(missing.is_none());

    cleanup(conn, summary_id, region_id);
}

/// 2. Inserting the same `(hash, metric, eval_version)` twice must not
/// error — the second call must return the *existing* row via the
/// `ON CONFLICT DO NOTHING` + re-SELECT fallback.
#[tokio::test]
async fn insert_score_idempotent_on_unique_conflict() {
    if !integration_enabled() {
        eprintln!("skipping: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let url = get_test_db_url();
    let pool = raw_pool();
    let conn = &mut pool.get().unwrap();
    let (summary_id, region_id) =
        seed_region_and_summary(conn, Some("body"), "idempotent_test_region");

    let pg = EvalsPostgresql::new();
    let hash = unique_tag("hash");
    let metric = "claim_groundedness".to_string();
    let eval_version = unique_tag("v");

    let new = NewEvalScore {
        summary_id,
        summary_hash: hash.clone(),
        metric: metric.clone(),
        score: 0.5,
        judge_model: None,
        details: None,
        eval_version: eval_version.clone(),
    };

    let first = pg
        .insert_score(&url, new.clone())
        .await
        .expect("first insert");
    let second = pg
        .insert_score(&url, new.clone())
        .await
        .expect("second insert should not error on conflict");

    // ON CONFLICT DO NOTHING + re-SELECT must return the row we originally
    // inserted; the `id` is server-generated so equality here proves the
    // "return existing row" contract.
    assert_eq!(first.id, second.id);
    assert_eq!(first.summary_hash, second.summary_hash);
    assert_eq!(first.metric, second.metric);
    assert_eq!(first.eval_version, second.eval_version);

    // And there's exactly one row in the DB.
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        c: i64,
    }
    let counts: Vec<Count> = diesel::sql_query(
        "SELECT COUNT(*)::bigint AS c FROM eval_scores
         WHERE summary_hash = $1 AND metric = $2 AND eval_version = $3",
    )
    .bind::<diesel::sql_types::Text, _>(&hash)
    .bind::<diesel::sql_types::Text, _>(&metric)
    .bind::<diesel::sql_types::Text, _>(&eval_version)
    .load(conn)
    .expect("count");
    assert_eq!(counts[0].c, 1);

    cleanup(conn, summary_id, region_id);
}

/// 3. `get_summary` returns the seeded row with all fields intact.
#[tokio::test]
async fn get_summary_roundtrip() {
    if !integration_enabled() {
        eprintln!("skipping: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let url = get_test_db_url();
    let pool = raw_pool();
    let conn = &mut pool.get().unwrap();
    let (summary_id, region_id) =
        seed_region_and_summary(conn, Some("hippocampus summary body"), "get_summary_test");

    // Set an acronym by updating the row post-seed (keeps the seed helper simple).
    diesel::sql_query("UPDATE region_summary SET acronym = $1 WHERE id = $2")
        .bind::<diesel::sql_types::Text, _>("HPC")
        .bind::<diesel::sql_types::Uuid, _>(summary_id)
        .execute(conn)
        .expect("update acronym");

    let pg = EvalsPostgresql::new();
    let got = pg
        .get_summary(&url, summary_id)
        .await
        .expect("get_summary")
        .expect("summary row should exist");

    assert_eq!(got.id, summary_id);
    assert_eq!(got.region_id, region_id);
    assert_eq!(got.name, "get_summary_test");
    assert_eq!(got.acronym.as_deref(), Some("HPC"));
    assert_eq!(got.summary, "hippocampus summary body");

    // Missing summary returns `None` cleanly.
    let missing = pg
        .get_summary(&url, Uuid::new_v4())
        .await
        .expect("get_summary missing");
    assert!(missing.is_none());

    cleanup(conn, summary_id, region_id);
}

/// 4. Seed a summary + 2 chunks in `brain_region_embeddings`, then assert
/// `load_chunks_by_ids` returns exactly those chunks (by chunk_text).
#[tokio::test]
async fn get_summary_with_chunks_returns_chunks() {
    if !integration_enabled() {
        eprintln!("skipping: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let url = get_test_db_url();
    let pool = raw_pool();
    let conn = &mut pool.get().unwrap();
    let (summary_id, region_id) =
        seed_region_and_summary(conn, Some("body"), "chunks_test_region");

    // Raw SQL to bypass the pgvector diesel type; `[0,0,...]`-style literal
    // is parsed by pgvector at cast time. 1536 dimensions matches the
    // schema; we'll use a shorthand via `array_fill`.
    let chunk_ids: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
    for (i, cid) in chunk_ids.iter().enumerate() {
        diesel::sql_query(
            "INSERT INTO brain_region_embeddings
                (id, region_id, summary_id, chunk_index, chunk_text, embedding)
             VALUES ($1, $2, $3, $4, $5,
                     array_fill(0.0::real, ARRAY[1536])::vector)",
        )
        .bind::<diesel::sql_types::Uuid, _>(*cid)
        .bind::<diesel::sql_types::Integer, _>(region_id)
        .bind::<diesel::sql_types::Uuid, _>(summary_id)
        .bind::<diesel::sql_types::Integer, _>(i as i32)
        .bind::<diesel::sql_types::Text, _>(format!("chunk text #{i}"))
        .execute(conn)
        .expect("insert chunk");
    }

    let pg = EvalsPostgresql::new();
    let rows = pg
        .load_chunks_by_ids(&url, &chunk_ids)
        .await
        .expect("load_chunks_by_ids");
    assert_eq!(rows.len(), 2);
    let got_texts: std::collections::HashSet<String> =
        rows.iter().map(|r| r.chunk_text.clone()).collect();
    assert!(got_texts.contains("chunk text #0"));
    assert!(got_texts.contains("chunk text #1"));
    for r in &rows {
        assert_eq!(r.summary_id, summary_id);
        assert!(chunk_ids.contains(&r.id));
    }

    // Empty input short-circuits with no DB round-trip.
    let empty = pg
        .load_chunks_by_ids(&url, &[])
        .await
        .expect("empty load_chunks_by_ids");
    assert!(empty.is_empty());

    cleanup(conn, summary_id, region_id);
}

/// 5. Each `RunState` phase serialises to JSONB, round-trips through
/// `save_run_state` / `load_run_state` bit-exactly. Guards the JSONB
/// schema of `eval_run_state.state`.
#[tokio::test]
async fn save_run_state_and_load_run_state_roundtrip() {
    if !integration_enabled() {
        eprintln!("skipping: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let url = get_test_db_url();
    let pool = raw_pool();
    let conn = &mut pool.get().unwrap();
    let (summary_id, region_id) =
        seed_region_and_summary(conn, Some("body"), "run_state_test_region");

    let pg = EvalsPostgresql::new();
    let eval_version = unique_tag("v");

    // The state machine's `RunState` lives in evals-services with strum
    // derives / serde tag = "phase". We don't want to depend on the
    // services crate just to produce each variant's wire form, so we
    // hand-build the JSONB shape per-phase (matches `state_machine.rs`).
    // Each variant is placed under its own run_id so we can round-trip
    // all of them in one test.
    let chunk_id = Uuid::new_v4();
    let chunk_row = ChunkRow {
        id: chunk_id,
        summary_id,
        chunk_index: 0,
        chunk_text: "cited chunk body".to_string(),
    };
    let mut cited_chunks: HashMap<Uuid, ChunkRow> = HashMap::new();
    cited_chunks.insert(chunk_id, chunk_row.clone());

    // `serde_json::to_value(&cited_chunks)` serialises a
    // `HashMap<Uuid, ChunkRow>` as a JSON object keyed by uuid strings.
    let cited_chunks_json = serde_json::to_value(&cited_chunks).unwrap();

    let variants: Vec<(&str, serde_json::Value)> = vec![
        (
            "awaiting_claims",
            serde_json::json!({ "phase": "awaiting_claims" }),
        ),
        (
            "awaiting_claim_embed",
            serde_json::json!({
                "phase": "awaiting_claim_embed",
                "claims": [],
                "idx": 0,
                "reports": []
            }),
        ),
        (
            "awaiting_claim_judge",
            serde_json::json!({
                "phase": "awaiting_claim_judge",
                "claims": [],
                "idx": 0,
                "reports": [],
                "retrieved": []
            }),
        ),
        (
            "awaiting_rubric",
            serde_json::json!({ "phase": "awaiting_rubric", "claims": null }),
        ),
        (
            "awaiting_citation_support",
            serde_json::json!({
                "phase": "awaiting_citation_support",
                "claims": [],
                "claim_idx": 0,
                "cite_idx": 0,
                "cited_chunks": cited_chunks_json,
                "issues": [],
                "support_supported": 0,
                "support_partial": 0,
                "support_unsupported": 0,
                "support_contradicted": 0,
                "totals": {
                    "total_claims": 0,
                    "claims_with_citation": 0,
                    "total_citations": 0,
                    "existing_citations": 0,
                    "in_scope_citations": 0
                },
                "support_calls_issued": 0,
                "truncated": false
            }),
        ),
        ("done", serde_json::json!({ "phase": "done" })),
    ];

    for (label, state_value) in variants.iter() {
        let run_id = pg
            .insert_run_state(&url, summary_id, &eval_version, state_value, None, None)
            .await
            .unwrap_or_else(|e| panic!("insert_run_state[{label}]: {e:?}"));

        // Round-trip 1: immediate load after insert.
        let loaded = pg
            .load_run_state(&url, run_id)
            .await
            .unwrap_or_else(|e| panic!("load_run_state[{label}]: {e:?}"))
            .unwrap_or_else(|| panic!("row missing after insert[{label}]"));
        let (loaded_summary_id, loaded_ver, loaded_state, loaded_pending) = loaded;
        assert_eq!(loaded_summary_id, summary_id, "[{label}] summary_id");
        assert_eq!(loaded_ver, eval_version, "[{label}] eval_version");
        assert_eq!(loaded_state, *state_value, "[{label}] state JSONB mismatch");
        assert!(loaded_pending.is_none(), "[{label}] pending");

        // Round-trip 2: save with a pending step id, reload, assert.
        let new_pending = Uuid::new_v4();
        pg.save_run_state(
            &url,
            run_id,
            state_value,
            Some(new_pending),
            Some("/llm/call"),
        )
        .await
        .unwrap_or_else(|e| panic!("save_run_state[{label}]: {e:?}"));

        let reloaded = pg
            .load_run_state(&url, run_id)
            .await
            .unwrap()
            .expect("row present after save");
        assert_eq!(reloaded.2, *state_value, "[{label}] state after save");
        assert_eq!(
            reloaded.3,
            Some(new_pending),
            "[{label}] pending_step_id after save"
        );

        // Special spot-check: for AwaitingCitationSupport, also verify that
        // `cited_chunks` is a HashMap<Uuid, ChunkRow> and round-trips back
        // through serde. This is the invariant the plan calls out.
        if *label == "awaiting_citation_support" {
            let reparsed: HashMap<Uuid, ChunkRow> =
                serde_json::from_value(reloaded.2.get("cited_chunks").unwrap().clone())
                    .expect("cited_chunks deserialises back to HashMap<Uuid, ChunkRow>");
            assert_eq!(reparsed.len(), 1);
            let got = reparsed.get(&chunk_id).expect("chunk present");
            assert_eq!(got.id, chunk_row.id);
            assert_eq!(got.summary_id, chunk_row.summary_id);
            assert_eq!(got.chunk_index, chunk_row.chunk_index);
            assert_eq!(got.chunk_text, chunk_row.chunk_text);
        }

        pg.delete_run_state(&url, run_id)
            .await
            .unwrap_or_else(|e| panic!("delete_run_state[{label}]: {e:?}"));
        let gone = pg.load_run_state(&url, run_id).await.unwrap();
        assert!(gone.is_none(), "[{label}] row should be deleted");
    }

    // `delete_run_states_for_summary` also works as a no-op when empty.
    pg.delete_run_states_for_summary(&url, summary_id, &eval_version)
        .await
        .expect("delete_run_states_for_summary");

    cleanup(conn, summary_id, region_id);
}

/// 6. Seed three scores with varying values; assert `get_worst_offenders`
/// returns them ordered ASC by score and honours `limit`.
#[tokio::test]
async fn list_worst_offenders_orders_by_score_asc() {
    if !integration_enabled() {
        eprintln!("skipping: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let url = get_test_db_url();
    let pool = raw_pool();
    let conn = &mut pool.get().unwrap();

    // Three distinct summaries so the three scores don't collide on
    // `(summary_id, metric, eval_version)` would-be uniqueness (not that
    // eval_scores enforces that, but it's cleaner).
    let mut seeded: Vec<(Uuid, i32)> = Vec::new();
    for i in 0..3 {
        let (sid, rid) =
            seed_region_and_summary(conn, Some("body"), &format!("worst_test_{i}"));
        seeded.push((sid, rid));
    }

    let pg = EvalsPostgresql::new();
    let metric = unique_tag("m");
    let eval_version = unique_tag("v");
    let scores = [0.9_f32, 0.1, 0.5];
    for ((sid, _), sc) in seeded.iter().zip(scores.iter()) {
        let hash = unique_tag("hash");
        pg.insert_score(
            &url,
            NewEvalScore {
                summary_id: *sid,
                summary_hash: hash,
                metric: metric.clone(),
                score: *sc,
                judge_model: None,
                details: None,
                eval_version: eval_version.clone(),
            },
        )
        .await
        .expect("insert_score");
    }

    let worst = pg
        .get_worst_offenders(&url, &metric, &eval_version, 10)
        .await
        .expect("get_worst_offenders");
    assert_eq!(worst.len(), 3, "expected exactly 3 rows");

    // Assert ascending score order.
    for pair in worst.windows(2) {
        assert!(
            pair[0].score <= pair[1].score,
            "expected ASC ordering: {} !<= {}",
            pair[0].score,
            pair[1].score
        );
    }
    assert!((worst[0].score - 0.1).abs() < 1e-6);
    assert!((worst[2].score - 0.9).abs() < 1e-6);
    for r in &worst {
        assert_eq!(r.metric, metric);
        assert_eq!(r.eval_version, eval_version);
        assert!(r.region_name.is_some());
    }

    // `limit` respected.
    let limited = pg
        .get_worst_offenders(&url, &metric, &eval_version, 2)
        .await
        .expect("get_worst_offenders limited");
    assert_eq!(limited.len(), 2);
    assert!((limited[0].score - 0.1).abs() < 1e-6);

    for (sid, rid) in seeded {
        cleanup(conn, sid, rid);
    }
}

/// 7. Seed scores under two different `eval_version`s; aggregate filtered
/// by one version must only see that version's rows.
#[tokio::test]
async fn aggregate_filters_by_eval_version() {
    if !integration_enabled() {
        eprintln!("skipping: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let url = get_test_db_url();
    let pool = raw_pool();
    let conn = &mut pool.get().unwrap();
    let (summary_id, region_id) =
        seed_region_and_summary(conn, Some("body"), "aggregate_test_region");

    let pg = EvalsPostgresql::new();
    let metric = unique_tag("m");
    let version_a = unique_tag("va");
    let version_b = unique_tag("vb");

    // 2 rows under version_a (score 0.4, 0.8), 1 row under version_b (0.2).
    for (score, ver) in [
        (0.4_f32, &version_a),
        (0.8, &version_a),
        (0.2, &version_b),
    ] {
        pg.insert_score(
            &url,
            NewEvalScore {
                summary_id,
                summary_hash: unique_tag("hash"),
                metric: metric.clone(),
                score,
                judge_model: None,
                details: None,
                eval_version: ver.clone(),
            },
        )
        .await
        .expect("insert_score");
    }

    let agg_a = pg
        .get_eval_aggregate(&url, &version_a)
        .await
        .expect("aggregate a");
    let stats_a = agg_a
        .per_metric
        .get(&metric)
        .expect("metric stats present under version_a");
    assert_eq!(stats_a.count, 2, "version_a should see exactly 2 rows");
    assert!(
        (stats_a.min - 0.4).abs() < 1e-4,
        "min={} expected ~0.4",
        stats_a.min
    );
    assert!(
        (stats_a.max - 0.8).abs() < 1e-4,
        "max={} expected ~0.8",
        stats_a.max
    );

    let agg_b = pg
        .get_eval_aggregate(&url, &version_b)
        .await
        .expect("aggregate b");
    let stats_b = agg_b
        .per_metric
        .get(&metric)
        .expect("metric stats present under version_b");
    assert_eq!(stats_b.count, 1);
    assert!((stats_b.min - 0.2).abs() < 1e-4);

    // version_a should NOT see the version_b row.
    assert!(
        (stats_a.min - 0.2).abs() > 1e-4,
        "aggregate bled across eval_versions"
    );

    cleanup(conn, summary_id, region_id);
}

/// 8. Full `eval_runs` lifecycle: `Queued` → `Running` → `Complete`.
/// After each transition, the row is queryable and the terminal row's
/// `completed_at` is set.
#[tokio::test]
async fn eval_runs_lifecycle() {
    if !integration_enabled() {
        eprintln!("skipping: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let url = get_test_db_url();
    let pool = raw_pool();
    let conn = &mut pool.get().unwrap();
    let (summary_id, region_id) =
        seed_region_and_summary(conn, Some("body"), "runs_lifecycle_region");

    let pg = EvalsPostgresql::new();
    let eval_version = unique_tag("v");

    // Queued
    let queued = pg
        .upsert_run(&url, summary_id, &eval_version, EvalRunStatus::Queued, None)
        .await
        .expect("upsert_run queued");
    assert_eq!(queued.status, EvalRunStatus::Queued);
    assert!(queued.started_at.is_none());
    assert!(queued.completed_at.is_none());

    // Running
    let running = pg
        .upsert_run(
            &url,
            summary_id,
            &eval_version,
            EvalRunStatus::Running,
            None,
        )
        .await
        .expect("upsert_run running");
    assert_eq!(running.id, queued.id, "same (summary, version) → same row");
    assert_eq!(running.status, EvalRunStatus::Running);
    assert!(running.started_at.is_some(), "Running should set started_at");
    assert!(running.completed_at.is_none());

    // Complete
    let complete = pg
        .upsert_run(
            &url,
            summary_id,
            &eval_version,
            EvalRunStatus::Complete,
            None,
        )
        .await
        .expect("upsert_run complete");
    assert_eq!(complete.id, queued.id);
    assert_eq!(complete.status, EvalRunStatus::Complete);
    assert!(
        complete.started_at.is_some(),
        "Complete must preserve started_at from Running"
    );
    assert!(complete.completed_at.is_some());

    // Query by eval_version — there's exactly one row.
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        c: i64,
    }
    let counts: Vec<Count> = diesel::sql_query(
        "SELECT COUNT(*)::bigint AS c FROM eval_runs WHERE eval_version = $1",
    )
    .bind::<diesel::sql_types::Text, _>(&eval_version)
    .load(conn)
    .expect("count eval_runs");
    assert_eq!(counts[0].c, 1, "exactly one row under this eval_version");

    cleanup(conn, summary_id, region_id);
}
