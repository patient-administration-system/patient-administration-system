//! Patient Administration System (PAS)
//!
//! A hospital Patient Administration System built with Rust.
//!
//! This library provides the administrative system of record for healthcare
//! workflow — identity, ADT (admission/discharge/transfer), scheduling,
//! waitlists, resources (wards/beds), communications, and billing. It is
//! deliberately non-clinical: diagnoses, prescriptions, vitals and lab
//! results are out of scope.
//!
//! Capabilities:
//! - ADT state machine with transactional bed allocation
//! - Slot-based scheduling with overlap detection
//! - Waitlist + RTT (Referral-To-Treatment) clock arithmetic
//! - Ward/Room/Bed resource model with status transitions
//! - Tera-rendered patient correspondence
//! - Episode-of-care billing (accounts, charges, invoices, payments)
//! - RESTful API via Axum + OpenAPI via Utoipa
//! - HL7 FHIR R5 surface for interoperability
//! - PostgreSQL persistence via SeaORM
//! - Transactional outbox for domain events
//! - Distributed tracing and observability via OpenTelemetry

#![cfg_attr(not(test), forbid(unsafe_code))]
// Production code handles every failure; tests may unwrap (QUAL-12).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
// Module declarations
pub mod adt;
pub mod analytics;
pub mod api;
pub mod backfill;
pub mod billing;
pub mod clock;
pub mod communication;
pub mod config;
pub mod contact;
pub mod db;
pub mod error;
pub mod experiments;
pub mod follow_up;
pub mod follow_up_delay;
pub mod history;
pub mod hl7v2;
pub mod identity;
pub mod inbound;
pub mod ivr;
pub mod jobs;
pub mod link_token;
pub mod models;
pub mod needs_review;
pub mod noshow;
pub mod observability;
pub mod paging;
pub mod printing;
pub mod privacy;
pub mod rebooking;
pub mod receipts;
pub mod referrals;
pub mod reply_links;
pub mod reports;
pub mod resources;
pub mod scheduling;
pub mod standby;
pub mod streaming;
pub mod support;
pub mod token_issuer;
pub mod tolerance;
pub mod transport;
pub mod upstream;
pub mod validation;
pub mod virtual_visits;
pub mod waitlist;
pub mod waits;
pub mod writeback;

// Re-exports
pub use error::{Error, Result};

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_imports() {
        // Verify key types are accessible
        let _: fn() -> crate::Result<()> = || Ok(());
    }
}
