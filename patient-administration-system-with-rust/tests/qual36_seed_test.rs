//! QUAL-36: `pas-seed` creates the demo reference data and is safe to run twice. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use patient_administration_system::db::repositories::bed::BedRepository;
use std::process::Command;

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_second_run_changes_nothing_and_the_ward_has_its_three_beds() {
    let db = common::build_state(&common::database_url().unwrap())
        .await
        .db;
    let first = patient_administration_system::seed::run(&db).await.unwrap();
    let second = patient_administration_system::seed::run(&db).await.unwrap();
    assert_eq!(second.created, 0, "everything was already there");
    assert_eq!(
        (
            first.facility_id,
            first.ward_id,
            first.room_id,
            &first.bed_ids
        ),
        (
            second.facility_id,
            second.ward_id,
            second.room_id,
            &second.bed_ids
        )
    );
    let beds = BedRepository::list_by_ward(&db, second.ward_id)
        .await
        .unwrap();
    assert_eq!(beds.len(), 3);
    let mut codes: Vec<_> = beds.iter().map(|b| b.code.clone()).collect();
    codes.sort();
    assert_eq!(codes, patient_administration_system::seed::BED_CODES);
}

#[test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
fn the_binary_runs_twice_and_says_it_seeds_no_people() {
    let url = common::database_url().unwrap();
    for _ in 0..2 {
        let out = Command::new(env!("CARGO_BIN_EXE_pas-seed"))
            .env_clear()
            .env("DATABASE_URL", &url)
            .output()
            .expect("run pas-seed");
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{text}");
        assert!(text.contains("ward_id="), "{text}");
        assert!(text.contains("people and staff are not seeded"), "{text}");
    }
}

#[test]
fn without_a_database_url_it_stops_with_a_message() {
    let out = Command::new(env!("CARGO_BIN_EXE_pas-seed"))
        .env_clear()
        .output()
        .expect("run pas-seed");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("DATABASE_URL is required"));
}
