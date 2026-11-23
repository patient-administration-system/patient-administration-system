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
        clippy::unwrap_in_result,
        // RCA-ENF-3: an arithmetic operation that can overflow is checked, or carries a written proof (RCA-EXC-2).
        clippy::arithmetic_side_effects,
        // RCA-CONV-1, RCA-CONV-2: a numeric conversion is lossless (`From`), checked (`TryFrom`), or carries a written proof.
        clippy::as_conversions
    )
)]

//! patient-administration-system-frontend — Axum and [axa] front-end for the Patient Administration System.
//!
//! Architecture:
//!
//! - The PAS Axum binary (`patient-administration-system`) is the system of
//!   record. It owns the REST + FHIR + HL7 v2 surface and writes to the
//!   `patients` / `encounters` / `admissions` / … tables.
//! - This crate (`patient-administration-system-frontend`) is a separate Axum service that:
//!   - Renders Tera templates dressed in [Lily Design System][lily] markup.
//!   - Serves HTMX-powered live views.
//!   - Reads from the same PostgreSQL database via sqlx.
//!   - Calls the PAS REST API for writes that go through the service layer
//!     (audit + outbox).
//!
//! The two services share migrations via the [`migration`] crate, so a single `pas-migrate up` keeps
//! both schemas in lock-step.
//!
//! [axa]: https://github.com/publicprojects-io/axa
//! [lily]: https://github.com/LilyDesignSystem/lily

pub mod app;
pub mod controllers;
pub mod csrf;
pub mod models;
pub mod services;
pub mod tpl;
pub mod views;
