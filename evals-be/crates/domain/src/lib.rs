//! Domain types for the evals-be service.
//!
//! Owns:
//! - `EvalScore`, `EvalRun`, `EvalRunStatus`: persistent rows in eval_scores / eval_runs.
//! - `EvalMetric`: enum of all known metric keys (single source of truth, no string typos).
//! - `ConfigKey`: tunable knobs surfaced via the future config API.
//! - `compute_hash`: SHA-256 hex digest used as the cache key.
//!
//! Re-exports the shared brainatlas eval wire types (`ClaimsResponse`,
//! `GroundednessVerdict`, `RubricScores`) so consumers can pull everything
//! eval-related from one place.

mod config;
mod evals;
mod hash;

pub use config::*;
pub use evals::*;
pub use hash::*;

// Re-export the shared eval wire types from brainatlas-be/domain so evals-be
// consumers can build them without an extra dependency.
pub use brainatlas_domain::{
    Claim, ClaimsResponse, GroundednessLabel, GroundednessVerdict, RubricCriterion, RubricScores,
};
