#![cfg_attr(not(test), forbid(unsafe_code))]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

//! patient-administration-system-frontend binary entry point.
//!
//! All wiring lives in `app.rs`; this file is intentionally tiny.

use axa::cli::Cli;
use clap::Parser;
use patient_administration_system_frontend::app::App;

#[tokio::main]
async fn main() -> axa::Result<()> {
    App::run(Cli::parse()).await
}
