//! Service-layer abstractions: traits the `app` layer talks to.
//!
//! Concrete impls live in the `infra` crate (Postgres + brainatlas HTTP client).
//! Tests can supply mocks of these traits without dragging in DB/network.

mod cache;
mod error;
mod infra;
pub mod groundedness;
pub mod rubric;
pub mod structural;

pub use cache::*;
pub use error::*;
pub use infra::*;
