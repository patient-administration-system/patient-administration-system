//! `pas-seed`: creates the demo reference data (one facility, ward, room and three beds) if it is missing.
//! Safe to run twice. It seeds no people: they live in `person-service` and `worker-service` (see
//! [`patient_administration_system::seed`]). Needs `DATABASE_URL` and a migrated database.
//!
//! It runs only in development or test, as axa's `Task::development_only` does, so that a made-up ward cannot
//! land among real ones. The environment is `APP_ENV`; with none set, a debug build is development and a
//! release build is production.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(not(test), forbid(unsafe_code))]

use axa_environment::Environment;
use patient_administration_system::config::env;

#[tokio::main]
async fn main() {
    let environment = Environment::from_env();
    if environment.is_production_like() {
        eprintln!(
            "pas-seed: refused: it runs only in development or test, and this is {environment} (set APP_ENV)"
        );
        std::process::exit(2);
    }
    let Some(url) = env::text("DATABASE_URL") else {
        eprintln!("pas-seed: DATABASE_URL is required");
        std::process::exit(2);
    };
    let db = match patient_administration_system::db::connect(&url).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("pas-seed: cannot connect to the database: {e}");
            std::process::exit(1);
        }
    };
    match patient_administration_system::seed::run(&db).await {
        Ok(s) => {
            println!(
                "pas-seed: {} record(s) created; ward_id={} bed_ids={}",
                s.created,
                s.ward_id,
                s.bed_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
            println!(
                "pas-seed: people and staff are not seeded; create them in person-service and worker-service"
            );
        }
        Err(e) => {
            eprintln!("pas-seed: failed: {e}");
            std::process::exit(1);
        }
    }
}
