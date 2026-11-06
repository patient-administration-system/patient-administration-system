#![cfg_attr(not(test), forbid(unsafe_code))]
// Production code handles every failure; tests may unwrap (QUAL-12).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

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
