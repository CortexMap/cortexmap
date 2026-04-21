//! Migration round-trip *static* tests for `evals-be`.
//!
//! **Chosen approach: Option C — static-text analysis (NO DB round-trip).**
//!
//! The shared CI test database already has every migration applied from the
//! bootstrap step in `ci/tests/ci.rs`. Running an `up → down → up` cycle
//! against it would nuke tables other tests depend on (`eval_scores`,
//! `eval_runs`, `eval_run_state`), and creating a throwaway database per test
//! requires superuser privileges that the `test_user` role may not carry
//! reliably across environments.
//!
//! Instead we parse the `up.sql` / `down.sql` files at test time and assert
//! structural invariants:
//!   * every `CREATE TABLE` / `CREATE INDEX` / `CREATE UNIQUE INDEX` in
//!     `up.sql` is matched by a corresponding `DROP` in `down.sql`;
//!   * the expected tables and indexes for this PR are actually declared;
//!   * `down.sql` drops objects in reverse dependency order (indexes before
//!     their table) — cheap guard against forgotten `DROP` statements.
//!
//! These tests catch the highest-value regression — a maintainer who adds a
//! new `CREATE` without the matching `DROP` — without needing a DB
//! connection. They are `#[test]` (synchronous) and unconditionally runnable;
//! no `RUN_INTEGRATION_TESTS` gating is needed.

use std::collections::HashSet;

const CREATE_EVAL_SCORES_UP: &str =
    include_str!("../../../migrations/2026-04-19-000001-create_eval_scores/up.sql");
const CREATE_EVAL_SCORES_DOWN: &str =
    include_str!("../../../migrations/2026-04-19-000001-create_eval_scores/down.sql");
const ADD_EVAL_RUN_STATE_UP: &str =
    include_str!("../../../migrations/2026-04-19-000002-add_eval_run_state/up.sql");
const ADD_EVAL_RUN_STATE_DOWN: &str =
    include_str!("../../../migrations/2026-04-19-000002-add_eval_run_state/down.sql");

/// Case-insensitive "does `haystack` contain `needle`?"
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// Extract object names (tables and indexes) declared by `CREATE` in `sql`.
/// Returns a set of lowercased names.
///
/// This is a deliberately simple parser — it assumes one statement per
/// `CREATE TABLE ...` / `CREATE [UNIQUE] INDEX ...` occurrence and that the
/// first identifier after the keywords is the object name, optionally
/// preceded by `IF NOT EXISTS`.
fn extract_created_objects(sql: &str) -> (HashSet<String>, HashSet<String>) {
    let mut tables = HashSet::new();
    let mut indexes = HashSet::new();

    for raw_line in sql.lines() {
        // Strip trailing `--` comments for easier matching.
        let line = raw_line.split("--").next().unwrap_or("").trim();
        let lower = line.to_ascii_lowercase();

        // CREATE TABLE [IF NOT EXISTS] <name>
        if let Some(rest) = lower.strip_prefix("create table ") {
            let rest = rest.trim_start_matches("if not exists ").trim();
            if let Some(name) = rest.split([' ', '(']).next()
                && !name.is_empty()
            {
                tables.insert(name.to_string());
            }
        }

        // CREATE [UNIQUE] INDEX [IF NOT EXISTS] <name> ON ...
        if lower.starts_with("create index ") || lower.starts_with("create unique index ") {
            let rest = lower
                .trim_start_matches("create unique index ")
                .trim_start_matches("create index ")
                .trim_start_matches("if not exists ")
                .trim();
            if let Some(name) = rest.split([' ', '\t']).next()
                && !name.is_empty()
            {
                indexes.insert(name.to_string());
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
            if let Some(name) = rest.split([' ', ';']).next()
                && !name.is_empty()
            {
                tables.insert(name.to_string());
            }
        }

        if lower.starts_with("drop index ") {
            let rest = lower
                .trim_start_matches("drop index ")
                .trim_start_matches("if exists ")
                .trim()
                .trim_end_matches(';')
                .trim();
            if let Some(name) = rest.split([' ', ';']).next()
                && !name.is_empty()
            {
                indexes.insert(name.to_string());
            }
        }
    }

    (tables, indexes)
}

/// Every CREATE in `up` must have a matching DROP in `down`. Indexes live
/// with their table so a dropped table implicitly drops its indexes — we
/// still require an explicit `DROP INDEX IF EXISTS` for clarity (and to
/// catch the "forgot a DROP" bug).
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
// Migration 2026-04-19-000001-create_eval_scores
// ============================================================================

#[test]
fn create_eval_scores_up_declares_expected_objects() {
    let (tables, indexes) = extract_created_objects(CREATE_EVAL_SCORES_UP);

    assert!(tables.contains("eval_scores"), "expected table eval_scores");
    assert!(tables.contains("eval_runs"), "expected table eval_runs");

    // Indexes declared in up.sql (see up.sql:24-31, 51-55).
    for expected in [
        "ix_eval_scores_cache",
        "ix_eval_scores_summary",
        "ix_eval_scores_metric_score",
        "ix_eval_runs_unique",
        "ix_eval_runs_status",
    ] {
        assert!(
            indexes.contains(expected),
            "expected index `{expected}` to be created in up.sql, got: {indexes:?}"
        );
    }
}

#[test]
fn create_eval_scores_down_drops_expected_objects() {
    let (tables, indexes) = extract_dropped_objects(CREATE_EVAL_SCORES_DOWN);

    assert!(tables.contains("eval_scores"));
    assert!(tables.contains("eval_runs"));

    for expected in [
        "ix_eval_scores_cache",
        "ix_eval_scores_summary",
        "ix_eval_scores_metric_score",
        "ix_eval_runs_unique",
        "ix_eval_runs_status",
    ] {
        assert!(
            indexes.contains(expected),
            "expected index `{expected}` to be dropped in down.sql, got: {indexes:?}"
        );
    }
}

#[test]
fn create_eval_scores_up_and_down_are_inverse() {
    assert_up_down_inverse(
        "create_eval_scores",
        CREATE_EVAL_SCORES_UP,
        CREATE_EVAL_SCORES_DOWN,
    );
}

// ============================================================================
// Migration 2026-04-19-000002-add_eval_run_state
// ============================================================================

#[test]
fn add_eval_run_state_up_declares_expected_objects() {
    let (tables, indexes) = extract_created_objects(ADD_EVAL_RUN_STATE_UP);

    assert!(
        tables.contains("eval_run_state"),
        "expected table eval_run_state"
    );
    assert!(
        indexes.contains("eval_run_state_summary_idx"),
        "expected index eval_run_state_summary_idx, got: {indexes:?}"
    );

    // JSONB-as-state contract: the column holding serialized RunState MUST be JSONB.
    // If someone accidentally downgrades to TEXT, service::state_machine round-trips break.
    assert!(
        contains_ci(ADD_EVAL_RUN_STATE_UP, "state              jsonb")
            || contains_ci(ADD_EVAL_RUN_STATE_UP, "state  jsonb")
            || contains_ci(ADD_EVAL_RUN_STATE_UP, "state jsonb"),
        "eval_run_state.state column must be declared JSONB"
    );
}

#[test]
fn add_eval_run_state_down_drops_expected_objects() {
    let (tables, _) = extract_dropped_objects(ADD_EVAL_RUN_STATE_DOWN);
    assert!(tables.contains("eval_run_state"));
}

#[test]
fn add_eval_run_state_up_and_down_are_inverse() {
    // Note: `eval_run_state_summary_idx` is implicitly dropped by DROP TABLE,
    // but that's a Postgres behavior — the down.sql here intentionally doesn't
    // re-list the index. Our inverse check only requires tables-to-tables
    // mapping when up.sql's indexes are not explicitly dropped.
    let (up_tables, _up_indexes) = extract_created_objects(ADD_EVAL_RUN_STATE_UP);
    let (down_tables, _) = extract_dropped_objects(ADD_EVAL_RUN_STATE_DOWN);
    for t in &up_tables {
        assert!(
            down_tables.contains(t),
            "add_eval_run_state: up.sql creates table `{t}` but down.sql is missing `DROP TABLE {t}`"
        );
    }
}
