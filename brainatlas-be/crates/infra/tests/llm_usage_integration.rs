//! Integration tests for the Diesel-backed `LlmPricingRepo` / `LlmUsageRepo`
//! implementations in `infra::llm_usage`, exercised through the public
//! `BrainAtlasInfra` facade (which delegates to `BrainAtlasLlmUsage`).
//!
//! Gated on `RUN_INTEGRATION_TESTS=1` and expects a Postgres instance reachable
//! at `TEST_DATABASE_URL` (defaults to the docker-compose test stack at
//! `postgresql://test_user:test_password@localhost:5433/test_db`).
//!
//! Each test uses UUID-scoped identifiers so the rows it inserts are unique to
//! that test, and cleans up after itself without touching the three seed rows
//! the `llm_pricing` migration inserts.
//!
//! See Plan Task 2.4 in `plans/2026-04-20-pr69-max-test-coverage-v1.md`. The
//! critical case in this file is `aggregate_correlation_id_prefix_escapes_like_wildcards`
//! which guards the LIKE-escape branch at `infra/src/llm_usage.rs:149-154`.

use bigdecimal::{BigDecimal, ToPrimitive};
use chrono::{DateTime, TimeZone, Utc};
use diesel::prelude::*;
use domain::{NewLlmCallUsage, UsageAggregate, UsageAggregateFilter};
use infra::{BrainAtlasInfra, InfraError};
use services::infra::{LlmPricingRepo, LlmUsageRepo};
use uuid::Uuid;

// --- env helpers --------------------------------------------------------------

fn integration_enabled() -> bool {
    std::env::var("RUN_INTEGRATION_TESTS").ok().as_deref() == Some("1")
}

fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://test_user:test_password@localhost:5433/test_db".to_string())
}

fn pg_conn() -> PgConnection {
    let url = test_database_url();
    PgConnection::establish(&url).expect("connect to test DB")
}

/// Macro to early-return a test when integration tests are disabled. We prefer
/// this over `#[ignore]` because the plan explicitly requires env-gated tests
/// so devs running `cargo test` locally without Docker don't get failures.
macro_rules! require_integration {
    () => {
        if !integration_enabled() {
            eprintln!("skipping: RUN_INTEGRATION_TESTS != 1");
            return;
        }
    };
}

// --- trait-qualified wrappers ------------------------------------------------
//
// `BrainAtlasInfra` implements ~half a dozen traits, and several of them have
// `type Error` as an associated type. When we call `repo.record(...).await`,
// rustc can't infer which trait's method we mean and bails with
// "type annotations needed". Wrapping the calls in these helpers pins the
// trait (and therefore the `Error` type) so the test bodies read cleanly.

async fn latest_for_model(
    repo: &BrainAtlasInfra,
    db: &str,
    model: &str,
) -> Result<Option<domain::LlmPricing>, InfraError> {
    <BrainAtlasInfra as LlmPricingRepo>::latest_for_model(repo, db, model).await
}

async fn record(
    repo: &BrainAtlasInfra,
    db: &str,
    row: NewLlmCallUsage,
) -> Result<(), InfraError> {
    <BrainAtlasInfra as LlmUsageRepo>::record(repo, db, row).await
}

async fn aggregate(
    repo: &BrainAtlasInfra,
    db: &str,
    filter: UsageAggregateFilter,
) -> Result<UsageAggregate, InfraError> {
    <BrainAtlasInfra as LlmUsageRepo>::aggregate(repo, db, filter).await
}

// --- cleanup helpers ---------------------------------------------------------

/// Delete any pricing rows we inserted for the given model (identified by a
/// unique model name containing a UUID, never a seed model).
fn cleanup_pricing_for_model(model: &str) {
    let mut conn = pg_conn();
    diesel::sql_query("DELETE FROM llm_pricing WHERE model = $1")
        .bind::<diesel::sql_types::Text, _>(model)
        .execute(&mut conn)
        .expect("delete test pricing rows");
}

/// Delete usage rows with `caller_tag = $1`. Tests stamp each row they insert
/// with a UUID-based caller tag so cleanup never bleeds into other tests.
fn cleanup_usage_by_caller_tag(tag: &str) {
    let mut conn = pg_conn();
    diesel::sql_query("DELETE FROM llm_call_usage WHERE caller_tag = $1")
        .bind::<diesel::sql_types::Text, _>(tag)
        .execute(&mut conn)
        .expect("delete test usage rows");
}

// --- shared builders ---------------------------------------------------------

fn minimal_usage(endpoint: &str, model: &str, caller_tag: &str) -> NewLlmCallUsage {
    NewLlmCallUsage {
        endpoint: endpoint.to_string(),
        model: model.to_string(),
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        cost_usd: Some(0.000123_f64),
        correlation_id: None,
        region_id: None,
        summary_id: None,
        batch_id: None,
        caller_tag: Some(caller_tag.to_string()),
        request_id: None,
    }
}

/// Insert a pricing row with an explicit `effective_from` via raw SQL. Infra's
/// public API only reads from `llm_pricing`, so seeding has to go through SQL.
fn insert_pricing_row(
    model: &str,
    input_ppm: &str,
    output_ppm: &str,
    effective_from: DateTime<Utc>,
) {
    let mut conn = pg_conn();
    diesel::sql_query(
        "INSERT INTO llm_pricing (model, input_price_per_million, output_price_per_million, effective_from) \
         VALUES ($1, $2::numeric, $3::numeric, $4)",
    )
    .bind::<diesel::sql_types::Text, _>(model)
    .bind::<diesel::sql_types::Text, _>(input_ppm)
    .bind::<diesel::sql_types::Text, _>(output_ppm)
    .bind::<diesel::sql_types::Timestamptz, _>(effective_from)
    .execute(&mut conn)
    .expect("insert pricing row");
}

// =============================================================================
// Tests
// =============================================================================

#[tokio::test]
async fn latest_for_model_orders_by_effective_from_desc() {
    require_integration!();
    let model = format!("test/latest-{}", Uuid::new_v4());

    // Three rows, distinct effective_from timestamps. The one in 2030 must win
    // regardless of insertion order.
    insert_pricing_row(&model, "0.1", "0.2", Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap());
    insert_pricing_row(&model, "0.9", "1.8", Utc.with_ymd_and_hms(2030, 6, 15, 12, 0, 0).unwrap());
    insert_pricing_row(&model, "0.5", "1.0", Utc.with_ymd_and_hms(2027, 3, 10, 8, 0, 0).unwrap());

    let repo = BrainAtlasInfra::new();
    let pricing = latest_for_model(&repo, &test_database_url(), &model)
        .await
        .expect("latest_for_model ok")
        .expect("row present");

    assert_eq!(pricing.model, model);
    // The 2030 row wins, so input=0.9 / output=1.8.
    assert!((pricing.input_price_per_million - 0.9).abs() < 1e-9);
    assert!((pricing.output_price_per_million - 1.8).abs() < 1e-9);
    assert_eq!(
        pricing.effective_from,
        Utc.with_ymd_and_hms(2030, 6, 15, 12, 0, 0).unwrap(),
    );

    cleanup_pricing_for_model(&model);
}

#[tokio::test]
async fn latest_for_model_returns_none_for_unknown_model() {
    require_integration!();
    let model = format!("test/nonexistent-{}", Uuid::new_v4());

    let repo = BrainAtlasInfra::new();
    let out = latest_for_model(&repo, &test_database_url(), &model)
        .await
        .expect("ok");
    assert!(out.is_none(), "expected None for unknown model, got {:?}", out);
}

#[tokio::test]
async fn record_inserts_row_with_all_fields() {
    require_integration!();
    let tag = format!("tag-{}", Uuid::new_v4());
    let corr = format!("corr-{}", Uuid::new_v4());
    let req = format!("req-{}", Uuid::new_v4());
    let summary_id = Uuid::new_v4();
    let batch_id = Uuid::new_v4();

    let row = NewLlmCallUsage {
        endpoint: "chat".to_string(),
        model: "openai/gpt-4o-mini".to_string(),
        prompt_tokens: 111,
        completion_tokens: 222,
        total_tokens: 333,
        cost_usd: Some(0.004567_f64),
        correlation_id: Some(corr.clone()),
        region_id: Some(42),
        summary_id: Some(summary_id),
        batch_id: Some(batch_id),
        caller_tag: Some(tag.clone()),
        request_id: Some(req.clone()),
    };

    let repo = BrainAtlasInfra::new();
    record(&repo, &test_database_url(), row).await.expect("record ok");

    // Round-trip: read the row straight back via raw SQL to avoid coupling the
    // test to the aggregate path (which we test separately).
    let mut conn = pg_conn();

    #[derive(QueryableByName, Debug)]
    struct Raw {
        #[diesel(sql_type = diesel::sql_types::Text)]
        endpoint: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        model: String,
        #[diesel(sql_type = diesel::sql_types::Integer)]
        prompt_tokens: i32,
        #[diesel(sql_type = diesel::sql_types::Integer)]
        completion_tokens: i32,
        #[diesel(sql_type = diesel::sql_types::Integer)]
        total_tokens: i32,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Numeric>)]
        cost_usd: Option<BigDecimal>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        correlation_id: Option<String>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
        region_id: Option<i32>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
        summary_id: Option<Uuid>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
        batch_id: Option<Uuid>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        caller_tag: Option<String>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        request_id: Option<String>,
    }

    let got: Raw = diesel::sql_query(
        "SELECT endpoint, model, prompt_tokens, completion_tokens, total_tokens, \
         cost_usd, correlation_id, region_id, summary_id, batch_id, caller_tag, request_id \
         FROM llm_call_usage WHERE correlation_id = $1",
    )
    .bind::<diesel::sql_types::Text, _>(&corr)
    .get_result(&mut conn)
    .expect("select back");

    assert_eq!(got.endpoint, "chat");
    assert_eq!(got.model, "openai/gpt-4o-mini");
    assert_eq!(got.prompt_tokens, 111);
    assert_eq!(got.completion_tokens, 222);
    assert_eq!(got.total_tokens, 333);
    assert_eq!(got.correlation_id.as_deref(), Some(corr.as_str()));
    assert_eq!(got.region_id, Some(42));
    assert_eq!(got.summary_id, Some(summary_id));
    assert_eq!(got.batch_id, Some(batch_id));
    assert_eq!(got.caller_tag.as_deref(), Some(tag.as_str()));
    assert_eq!(got.request_id.as_deref(), Some(req.as_str()));

    // BigDecimal cost: convert back through f64 and compare with tolerance.
    // The column is numeric(14,8), so we can't expect bit-exact fidelity.
    let cost_f = got.cost_usd.expect("cost present").to_f64().expect("to_f64");
    assert!(
        (cost_f - 0.004567_f64).abs() < 1e-8,
        "cost round-trip mismatch: {}",
        cost_f
    );

    cleanup_usage_by_caller_tag(&tag);
}

#[tokio::test]
async fn record_handles_bigdecimal_precision_edges() {
    require_integration!();
    let tag = format!("edge-{}", Uuid::new_v4());
    let repo = BrainAtlasInfra::new();
    let db = test_database_url();

    // `numeric(14,8)` only holds values with 14 total digits and 8 after the
    // decimal point. `f64::MIN_POSITIVE` rounds to 0 after scale coercion; the
    // contract exercised here is "no panic, no error", not perfect fidelity.
    for cost in [Some(f64::MIN_POSITIVE), Some(0.0), Some(1.0e-8)] {
        let mut row = minimal_usage("chat", "openai/gpt-4o-mini", &tag);
        row.cost_usd = cost;
        // Unique correlation_id per row so nothing collides on indexes.
        row.correlation_id = Some(format!("edge-{}", Uuid::new_v4()));
        record(&repo, &db, row).await.expect("small-cost record");
    }

    // A "large" value that still fits in numeric(14,8) — ~999,999.99999999 is
    // the column ceiling. The plan asked for `1.0e18`, but that would be
    // rejected by Postgres, so we cap at a representable-but-large value. The
    // real goal is that `BigDecimal::from_f64` survives without panicking.
    let mut big_row = minimal_usage("chat", "openai/gpt-4o-mini", &tag);
    big_row.cost_usd = Some(123456.78901234_f64);
    big_row.correlation_id = Some(format!("edge-{}", Uuid::new_v4()));
    record(&repo, &db, big_row).await.expect("large-cost record");

    // Sanity: aggregate by caller_tag and confirm 4 rows were inserted.
    let filter = UsageAggregateFilter {
        caller_tag: Some(tag.clone()),
        ..Default::default()
    };
    let agg = aggregate(&repo, &db, filter).await.expect("aggregate ok");
    assert_eq!(agg.total_calls, 4, "expected 4 rows, got {}", agg.total_calls);

    cleanup_usage_by_caller_tag(&tag);
}

#[tokio::test]
async fn record_null_cost_when_pricing_missing() {
    require_integration!();
    let tag = format!("nullcost-{}", Uuid::new_v4());
    let corr = format!("corr-null-{}", Uuid::new_v4());

    let mut row = minimal_usage("chat", "openai/gpt-4o-mini", &tag);
    row.cost_usd = None;
    row.correlation_id = Some(corr.clone());

    let repo = BrainAtlasInfra::new();
    record(&repo, &test_database_url(), row).await.expect("record ok");

    // Verify the cost_usd column is truly NULL in the DB.
    #[derive(QueryableByName, Debug)]
    struct NullCheck {
        #[diesel(sql_type = diesel::sql_types::Bool)]
        is_null: bool,
    }
    let mut conn = pg_conn();
    let r: NullCheck = diesel::sql_query(
        "SELECT cost_usd IS NULL AS is_null FROM llm_call_usage WHERE correlation_id = $1",
    )
    .bind::<diesel::sql_types::Text, _>(&corr)
    .get_result(&mut conn)
    .expect("select");
    assert!(r.is_null, "cost_usd should be NULL");

    cleanup_usage_by_caller_tag(&tag);
}

/// The critical LIKE-escape test (Plan §2.4, line 221 of the plan).
///
/// Guards `infra/src/llm_usage.rs:149-154`: the prefix filter must escape `%`,
/// `_`, and `\` so that a user-supplied prefix cannot expand into a SQL
/// wildcard and match unrelated rows. If this test fails, `correlation_id_prefix`
/// leaks across tenants.
#[tokio::test]
async fn aggregate_correlation_id_prefix_escapes_like_wildcards() {
    require_integration!();
    // Unique per-run caller_tag so we can cleanly isolate our rows.
    let tag = format!("likeescape-{}", Uuid::new_v4());
    let repo = BrainAtlasInfra::new();
    let db = test_database_url();

    // Seed 5 rows. Four have the `batch:abc*` correlation_id family, one is a
    // distractor from another "tenant".
    let seeds = [
        "batch:abc%123",
        "batch:abc_def",
        "batch:abc\\def",
        "batch:abcXXX",
        "other:xyz",
    ];
    for corr in seeds.iter() {
        let mut row = minimal_usage("chat", "openai/gpt-4o-mini", &tag);
        row.correlation_id = Some((*corr).to_string());
        record(&repo, &db, row).await.expect("record seed");
    }

    // --- Prefix: "batch:abc%" ------------------------------------------------
    // If `%` were NOT escaped, this prefix would expand to `batch:abc%%` and
    // match all four `batch:abc*` rows. With correct escaping, it matches ONLY
    // the literal `batch:abc%123`.
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            correlation_id_prefix: Some("batch:abc%".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("aggregate ok");
    assert_eq!(
        agg.total_calls, 1,
        "prefix 'batch:abc%' should match ONLY 'batch:abc%123'; got {} rows (escape broken?)",
        agg.total_calls,
    );

    // --- Prefix: "batch:abc_" ------------------------------------------------
    // Unescaped, `_` would match any single char, catching both `batch:abc_def`
    // AND `batch:abcXXX` (first 10 chars 'batch:abcX'). With escaping, only the
    // literal underscore row matches.
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            correlation_id_prefix: Some("batch:abc_".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("aggregate ok");
    assert_eq!(
        agg.total_calls, 1,
        "prefix 'batch:abc_' should match ONLY 'batch:abc_def'; got {} rows (underscore escape broken?)",
        agg.total_calls,
    );

    // --- Prefix: "batch:abc\" (single raw backslash) -------------------------
    // The raw backslash needs to be escaped to `\\` in the LIKE pattern so
    // Postgres doesn't treat it as its LIKE-level escape character. A correct
    // implementation matches ONLY the row whose correlation_id starts with a
    // literal backslash.
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            correlation_id_prefix: Some("batch:abc\\".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("aggregate ok");
    assert_eq!(
        agg.total_calls, 1,
        "prefix 'batch:abc\\' should match ONLY 'batch:abc\\def'; got {} rows (backslash escape broken?)",
        agg.total_calls,
    );

    // --- Sanity: a real prefix ("batch:abcX") should still work --------------
    // Regression-guard the happy path so we don't over-escape.
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            correlation_id_prefix: Some("batch:abcX".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("aggregate ok");
    assert_eq!(
        agg.total_calls, 1,
        "prefix 'batch:abcX' should match 'batch:abcXXX' only; got {}",
        agg.total_calls,
    );

    // --- Sanity: unscoped prefix "batch:" catches all four batch rows --------
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            correlation_id_prefix: Some("batch:".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("aggregate ok");
    assert_eq!(
        agg.total_calls, 4,
        "prefix 'batch:' should match all four batch rows; got {}",
        agg.total_calls,
    );

    cleanup_usage_by_caller_tag(&tag);
}

#[tokio::test]
async fn aggregate_filters_by_model() {
    require_integration!();
    let tag = format!("bymodel-{}", Uuid::new_v4());
    let model_a = format!("testmodel/a-{}", Uuid::new_v4());
    let model_b = format!("testmodel/b-{}", Uuid::new_v4());
    let repo = BrainAtlasInfra::new();
    let db = test_database_url();

    // 3 rows for model_a, 2 for model_b.
    for _ in 0..3 {
        record(&repo, &db, minimal_usage("chat", &model_a, &tag))
            .await
            .expect("record a");
    }
    for _ in 0..2 {
        record(&repo, &db, minimal_usage("chat", &model_b, &tag))
            .await
            .expect("record b");
    }

    let agg_a = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            model: Some(model_a.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("aggregate a");
    assert_eq!(agg_a.total_calls, 3, "model_a should have 3 rows");

    let agg_b = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            model: Some(model_b.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("aggregate b");
    assert_eq!(agg_b.total_calls, 2, "model_b should have 2 rows");

    cleanup_usage_by_caller_tag(&tag);
}

#[tokio::test]
async fn aggregate_filters_by_since_and_until() {
    require_integration!();
    let tag = format!("window-{}", Uuid::new_v4());
    let repo = BrainAtlasInfra::new();
    let db = test_database_url();

    // We can't backdate `created_at` via the repo, so insert via raw SQL with
    // explicit timestamps.
    let mut conn = pg_conn();
    let rows = [
        Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap(),
    ];
    for ts in rows.iter() {
        diesel::sql_query(
            "INSERT INTO llm_call_usage (created_at, endpoint, model, prompt_tokens, \
             completion_tokens, total_tokens, caller_tag) \
             VALUES ($1, 'chat', 'openai/gpt-4o-mini', 1, 1, 2, $2)",
        )
        .bind::<diesel::sql_types::Timestamptz, _>(*ts)
        .bind::<diesel::sql_types::Text, _>(&tag)
        .execute(&mut conn)
        .expect("insert ts row");
    }

    // Window [2025-06-01, 2026-06-01] → only the 2026 row.
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            since: Some(Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap()),
            until: Some(Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(agg.total_calls, 1, "window should match only the mid row");

    // since only: 2026-01-01 onward → 2 rows (mid + new).
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            since: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(agg.total_calls, 2, "since=2026 should match mid+new");

    // until only: up to 2026-01-01 → 2 rows (old + mid, since `le` is inclusive).
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            until: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(agg.total_calls, 2, "until=2026 (inclusive) should match old+mid");

    cleanup_usage_by_caller_tag(&tag);
}

#[tokio::test]
async fn aggregate_filters_by_correlation_id_exact() {
    require_integration!();
    let tag = format!("exact-{}", Uuid::new_v4());
    let common = format!("run-{}", Uuid::new_v4());
    let repo = BrainAtlasInfra::new();
    let db = test_database_url();

    // Three rows share the same prefix but differ in suffix.
    let ids = [
        format!("{}-step1", common),
        format!("{}-step2", common),
        format!("{}-step1", common), // duplicate of step1 on purpose
    ];
    for id in ids.iter() {
        let mut r = minimal_usage("chat", "openai/gpt-4o-mini", &tag);
        r.correlation_id = Some(id.clone());
        record(&repo, &db, r).await.expect("record");
    }

    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            correlation_id: Some(format!("{}-step1", common)),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(
        agg.total_calls, 2,
        "exact match on -step1 should return the 2 step1 rows"
    );

    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            correlation_id: Some(format!("{}-step2", common)),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(agg.total_calls, 1, "exact match on -step2 should return 1");

    // And exact match for a never-used id returns 0.
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(tag.clone()),
            correlation_id: Some(format!("{}-nope", common)),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(agg.total_calls, 0);

    cleanup_usage_by_caller_tag(&tag);
}

#[tokio::test]
async fn aggregate_filters_by_region_id_summary_id_batch_id_caller_tag() {
    require_integration!();
    // We use a fresh scope tag so cleanup is clean, and a SECOND tag (`other`)
    // to prove the caller_tag filter excludes it.
    let tag = format!("scope-{}", Uuid::new_v4());
    let other = format!("other-{}", Uuid::new_v4());
    let repo = BrainAtlasInfra::new();
    let db = test_database_url();

    let region_a: i32 = 1001;
    let region_b: i32 = 2002;
    let summary_a = Uuid::new_v4();
    let summary_b = Uuid::new_v4();
    let batch_a = Uuid::new_v4();
    let batch_b = Uuid::new_v4();

    let make = |region, summary, batch, t: &str| {
        let mut r = minimal_usage("chat", "openai/gpt-4o-mini", t);
        r.region_id = Some(region);
        r.summary_id = Some(summary);
        r.batch_id = Some(batch);
        r
    };

    // 2 rows with (region_a, summary_a, batch_a, tag)
    record(&repo, &db, make(region_a, summary_a, batch_a, &tag))
        .await
        .expect("rec");
    record(&repo, &db, make(region_a, summary_a, batch_a, &tag))
        .await
        .expect("rec");
    // 1 row (region_b, summary_b, batch_b, tag)
    record(&repo, &db, make(region_b, summary_b, batch_b, &tag))
        .await
        .expect("rec");
    // 1 row identical to the first but with a DIFFERENT caller_tag
    record(&repo, &db, make(region_a, summary_a, batch_a, &other))
        .await
        .expect("rec");

    // region_id = region_a → should include 2 rows with our `tag` AND 1 with
    // `other`, total 3.
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            region_id: Some(region_a),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(
        agg.total_calls, 3,
        "region_a alone should match our 3 seeded rows (2 `tag` + 1 `other`)"
    );

    // region_id = region_a AND caller_tag = tag → 2 rows.
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            region_id: Some(region_a),
            caller_tag: Some(tag.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(
        agg.total_calls, 2,
        "region_a + caller_tag should match exactly the 2 tagged rows"
    );

    // summary_id = summary_b → only the single row
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            summary_id: Some(summary_b),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(agg.total_calls, 1);

    // batch_id = batch_a → 3 rows
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            batch_id: Some(batch_a),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(agg.total_calls, 3);

    // caller_tag alone = `other` → 1 row
    let agg = aggregate(
        &repo,
        &db,
        UsageAggregateFilter {
            caller_tag: Some(other.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("agg");
    assert_eq!(agg.total_calls, 1);

    cleanup_usage_by_caller_tag(&tag);
    cleanup_usage_by_caller_tag(&other);
}
