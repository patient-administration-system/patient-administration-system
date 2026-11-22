//! Patient Administration System: domain logic and business rules.
//!
//! The models and their state lists, validation, identifiers, the clock, the settings registry, and the rules
//! that need neither the web nor the database. The server crate (`patient-administration-system`) owns the
//! routes, the storage and the integrations, and re-exports these modules at their old paths, so
//! `crate::models::...` there still works.
//!
//! Nothing here may use a web framework, an HTTP client, a database driver or an async runtime; the server's
//! `tests/domain_boundary_test.rs` checks the manifest.

#![cfg_attr(not(test), forbid(unsafe_code))]
// Production code handles every failure; tests may unwrap (QUAL-12).
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented,
        clippy::indexing_slicing,
        clippy::string_slice,
        clippy::panic_in_result_fn,
        clippy::unwrap_in_result
    )
)]

pub mod arith;
pub mod calendar;
pub mod clock;
pub mod communication;
pub mod config;
pub mod error;
pub mod identifiers;
pub mod inbound;
pub mod jsonx;
pub mod list_validation;
pub mod models;
pub mod noshow;
pub mod privacy;
pub mod referrals;
pub mod standby;
pub mod suspicions;
pub mod time;
pub mod tolerance;
pub mod triage;
pub mod validation;
pub mod work_queue;

pub use error::{Error, Result};
