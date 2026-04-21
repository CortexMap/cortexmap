//! Service-layer abstractions: traits the `app` layer talks to.
//!
//! Concrete impls live in the `infra` crate (Postgres only — evals-be is a
//! pure stateless state machine as of 2026-04-19, so there is no outbound
//! HTTP client here).

mod cache;
pub mod citations;
mod error;
mod infra;
pub mod state_machine;
pub mod structural;

pub use cache::*;
pub use error::*;
pub use infra::*;
