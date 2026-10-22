//! `pas-seed`: creates the demo reference data (one facility, ward, room and three beds) if it is missing.
//! Safe to run twice. It seeds no people: they live in `person-service` and `worker-service` (see
//! [`patient_administration_system::seed`]). Needs `DATABASE_URL` and a migrated database.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(not(test), forbid(unsafe_code))]

use patient_administration_system::config::env;

#[tokio::main]
async fn main() {
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
