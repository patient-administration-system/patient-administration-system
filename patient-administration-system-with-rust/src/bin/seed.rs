//! `pas-seed`: **not implemented**. It prints that and exits with status 1, so a script that expects demo data
//! stops instead of carrying on with an empty database.
//!
//! The PAS no longer holds patient or practitioner rows (identity lives in `person-service` and
//! `worker-service`). A real seed has to create people and workers there through `PersonClient::create` and
//! `WorkerClient::create`, then write the PAS-owned rows (facility, ward, room, bed, letter template) that
//! refer to the returned ids. See QUAL-36 in `tasks.md`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(not(test), forbid(unsafe_code))]
fn main() {
    eprintln!(
        "pas-seed is not implemented: it would have to create people and workers in person-service and \
         worker-service and then write the PAS-owned rows that refer to them (QUAL-36 in tasks.md). \
         Nothing was loaded."
    );
    std::process::exit(1);
}
