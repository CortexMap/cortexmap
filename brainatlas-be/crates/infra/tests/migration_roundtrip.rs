//! Migration round-trip *static* tests for `brainatlas-be`.
//!
//! **Chosen approach: Option C — static-text analysis (NO DB round-trip).**
//!
//! The shared CI test database already has every migration applied from
//! `ci/tests/ci.rs`'s bootstrap step. An actual `up → down → up` cycle
//! against it would destroy `llm_pricing` and `llm_call_usage` for all
//! concurrently-running tests, and creating a throwaway database per test
//! requires superuser privileges that are not guaranteed across
//! environments.
//!
//! Instead we parse the SQL files at test time and assert structural
//! invariants:
//!   * every `CREATE TABLE` / `CREATE INDEX` in `up.sql` has a matching
//!     `DROP` in `down.sql`;
//!   * the expected tables / indexes / seed rows / column types for this
//!     PR are actually declared.
//!
//! The migration `2026-04-20-000001-add_llm_pricing` has three seed rows at
//! `up.sql:25-30`; we assert each seeded model string appears verbatim.
//!
//! These tests catch the highest-value regression — a maintainer who adds a
//! new `CREATE` without the matching `DROP` — without needing DB access.
//! They are `#[test]` (synchronous) and unconditionally runnable.

use std::collections::HashSet;

const ADD_LLM_PRICING_UP: &str =
    include_str!("../../../migrations/2026-04-20-000001-add_llm_pricing/up.sql");
const ADD_LLM_PRICING_DOWN: &str =
    include_str!("../../../migrations/2026-04-20-000001-add_llm_pricing/down.sql");
const ADD_LLM_CALL_USAGE_UP: &str =
    include_str!("../../../migrations/2026-04-20-000002-add_llm_call_usage/up.sql");
const ADD_LLM_CALL_USAGE_DOWN: &str =
    include_str!("../../../migrations/2026-04-20-000002-add_llm_call_usage/down.sql");

/// Extract object names (tables and indexes) declared by `CREATE` in `sql`.
/// Returns (tables, indexes) as lowercased name sets. Tolerant of
/// `IF NOT EXISTS`, leading whitespace, and inline comments.
fn extract_created_objects(sql: &str) -> (HashSet<String>, HashSet<String>) {
    let mut tables = HashSet::new();
    let mut indexes = HashSet::new();

    for raw_line in sql.lines() {
        let line = raw_line.split("--").next().unwrap_or("").trim();
        let lower = line.to_ascii_lowercase();

        if let Some(rest) = lower.strip_prefix("create table ") {
            let rest = rest.trim_start_matches("if not exists ").trim();
            if let Some(name) = rest.split(|c: char| c == ' ' || c == '(').next() {
                if !name.is_empty() {
                    tables.insert(name.to_string());
                }
            }
        }

        if lower.starts_with("create index ")
            || lower.starts_with("create unique index ")
        {
            let rest = lower
                .trim_start_matches("create unique index ")
                .trim_start_matches("create index ")
                .trim_start_matches("if not exists ")
                .trim();
            if let Some(name) = rest.split(|c: char| c == ' ' || c == '\t').next() {
                if !name.is_empty() {
                    indexes.insert(name.to_string());
                }
            }
        }
    }

    (tables, indexes)
}

/// Extract object names referenced by `DROP TABLE` / `DROP INDEX` in `sql`.
fn extract_dropped_objects(sql: &str) -> (HashSet<String>, HashSet<String>) {
    let mut tables = HashSet::new();
    let mut indexes = HashSet::new();

    for raw_line in sql.lines() {
        let line = raw_line.split("--").next().unwrap_or("").trim();
        let lower = line.to_ascii_lowercase();

        if lower.starts_with("drop table ") {
            let rest = lower
                .trim_start_matches("drop table ")
                .trim_start_matches("if exists ")
                .trim()
                .trim_end_matches(';')
                .trim();
            if let Some(name) = rest.split(|c: char| c == ' ' || c == ';').next() {
                if !name.is_empty() {
                    tables.insert(name.to_string());
                }
            }
        }

        if lower.starts_with("drop index ") {
            let rest = lower
                .trim_start_matches("drop index ")
                .trim_start_matches("if exists ")
                .trim()
                .trim_end_matches(';')
                .trim();
            if let Some(name) = rest.split(|c: char| c == ' ' || c == ';').next() {
                if !name.is_empty() {
                    indexes.insert(name.to_string());
                }
            }
        }
    }

    (tables, indexes)
}

fn assert_up_down_inverse(migration_name: &str, up_sql: &str, down_sql: &str) {
    let (up_tables, up_indexes) = extract_created_objects(up_sql);
    let (down_tables, down_indexes) = extract_dropped_objects(down_sql);

    for t in &up_tables {
        assert!(
            down_tables.contains(t),
            "{migration_name}: up.sql creates table `{t}` but down.sql is missing a `DROP TABLE {t}`"
        );
    }
    for i in &up_indexes {
        assert!(
            down_indexes.contains(i),
            "{migration_name}: up.sql creates index `{i}` but down.sql is missing a `DROP INDEX {i}`"
        );
    }
}

// ============================================================================
// Migration 2026-04-20-000001-add_llm_pricing
// ============================================================================

#[test]
fn add_llm_pricing_up_declares_expected_objects() {
    let (tables, indexes) = extract_created_objects(ADD_LLM_PRICING_UP);

    assert!(
        tables.contains("llm_pricing"),
        "expected table llm_pricing, got: {tables:?}"
    );

    // up.sql:16-20 declares two indexes.
    for expected in ["idx_llm_pricing_model_effective", "idx_llm_pricing_model"] {
        assert!(
            indexes.contains(expected),
            "expected index `{expected}`, got: {indexes:?}"
        );
    }
}

/// Verify the three seed rows at `up.sql:25-30` are present in the migration
/// file. This guards the runbook dependency documented in
/// plans/2026-04-20-llm-cost-tracking-v1.md (Task 25).
#[test]
fn add_llm_pricing_up_contains_three_seed_rows() {
    // The seed INSERT block must mention each of these three models verbatim.
    // If a maintainer removes or renames one of these, cost computation for
    // that model silently falls through to the null-pricing branch.
    for model in [
        "openai/gpt-4o-mini",
        "openai/gpt-4o",
        "text-embedding-3-small",
    ] {
        assert!(
            ADD_LLM_PRICING_UP.contains(model),
            "expected seed model `{model}` in up.sql"
        );
    }

    // The INSERT must exist and use ON CONFLICT DO NOTHING so re-applying
    // the migration is idempotent.
    let up_lower = ADD_LLM_PRICING_UP.to_ascii_lowercase();
    assert!(
        up_lower.contains("insert into llm_pricing"),
        "expected an INSERT INTO llm_pricing in up.sql"
    );
    assert!(
        up_lower.contains("on conflict do nothing"),
        "llm_pricing seed INSERT must use ON CONFLICT DO NOTHING for idempotency"
    );
}

#[test]
fn add_llm_pricing_down_drops_expected_objects() {
    let (tables, indexes) = extract_dropped_objects(ADD_LLM_PRICING_DOWN);

    assert!(tables.contains("llm_pricing"));
    for expected in ["idx_llm_pricing_model_effective", "idx_llm_pricing_model"] {
        assert!(
            indexes.contains(expected),
            "expected index `{expected}` to be dropped, got: {indexes:?}"
        );
    }
}

#[test]
fn add_llm_pricing_up_and_down_are_inverse() {
    assert_up_down_inverse(
        "add_llm_pricing",
        ADD_LLM_PRICING_UP,
        ADD_LLM_PRICING_DOWN,
    );
}

// ============================================================================
// Migration 2026-04-20-000002-add_llm_call_usage
// ============================================================================

/// `up.sql` declares exactly 6 indexes (lines 31-47). The audit endpoint
/// query planner at `brainatlas-be/crates/infra/src/llm_usage.rs` relies on
/// each of these; dropping one silently regresses query performance.
const LLM_CALL_USAGE_EXPECTED_INDEXES: &[&str] = &[
    "idx_llm_call_usage_created_at",
    "idx_llm_call_usage_model_created_at",
    "idx_llm_call_usage_correlation_id",
    "idx_llm_call_usage_region_created_at",
    "idx_llm_call_usage_summary",
    "idx_llm_call_usage_batch",
];

#[test]
fn add_llm_call_usage_up_declares_expected_objects() {
    let (tables, indexes) = extract_created_objects(ADD_LLM_CALL_USAGE_UP);

    assert!(
        tables.contains("llm_call_usage"),
        "expected table llm_call_usage, got: {tables:?}"
    );

    // Contract: all six indexes must be present.
    assert_eq!(
        indexes.len(),
        LLM_CALL_USAGE_EXPECTED_INDEXES.len(),
        "expected exactly {} indexes on llm_call_usage, got {}: {:?}",
        LLM_CALL_USAGE_EXPECTED_INDEXES.len(),
        indexes.len(),
        indexes
    );

    for expected in LLM_CALL_USAGE_EXPECTED_INDEXES {
        assert!(
            indexes.contains(*expected),
            "expected index `{expected}` on llm_call_usage, got: {indexes:?}"
        );
    }
}

#[test]
fn add_llm_call_usage_down_drops_all_six_indexes_and_table() {
    let (tables, indexes) = extract_dropped_objects(ADD_LLM_CALL_USAGE_DOWN);

    assert!(
        tables.contains("llm_call_usage"),
        "expected DROP TABLE llm_call_usage in down.sql"
    );

    for expected in LLM_CALL_USAGE_EXPECTED_INDEXES {
        assert!(
            indexes.contains(*expected),
            "expected DROP INDEX `{expected}` in down.sql, got: {indexes:?}"
        );
    }
}

#[test]
fn add_llm_call_usage_up_and_down_are_inverse() {
    assert_up_down_inverse(
        "add_llm_call_usage",
        ADD_LLM_CALL_USAGE_UP,
        ADD_LLM_CALL_USAGE_DOWN,
    );
}
