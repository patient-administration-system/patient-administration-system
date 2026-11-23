#![cfg_attr(not(test), forbid(unsafe_code))]
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

//! patient-administration-system-frontend binary entry point.
//!
//! All wiring lives in `app.rs`; this file is intentionally tiny.

use patient_administration_system_frontend::app::App;

#[tokio::main]
async fn main() -> axa::Result<()> {
    patient_administration_system_frontend::services::ensure_tls_provider();
    let cli = axa::cli::parse(
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_DESCRIPTION"),
    );
    App::run(cli).await
}
