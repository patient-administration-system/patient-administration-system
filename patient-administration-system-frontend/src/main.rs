#![cfg_attr(not(test), forbid(unsafe_code))]
#![allow(
    clippy::result_large_err,
    reason = "loco_rs::Error is large by design; boxing every handler result costs more than it saves"
)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

//! patient-administration-system-frontend binary entry point.
//!
//! Boots the Loco-rs app. All wiring (routes, hooks, AppContext) lives in
//! `app.rs`; this file is intentionally tiny so the CLI surface stays
//! debuggable.

use loco_rs::cli;
use migration::Migrator;
use patient_administration_system_frontend::app::App;

#[tokio::main]
async fn main() -> loco_rs::Result<()> {
    cli::main::<App, Migrator>().await
}
