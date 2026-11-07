//! QUAL-48: a figure over the waiting list counts all of it, and a cut audit says so. Needs PostgreSQL:
//! `cargo test -- --ignored --test-threads=1`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::appointment::Appointment;
use patient_administration_system::reports::GroupBy;
use uuid::Uuid;

/// Adds `count` open pathways for `service` at once, each started `years` ago. [`remove_pathways`] clears them.
async fn add_pathways(s: &AppState, service: &str, count: i32, years: i32) {
    s.db.run(Stmt::new(
        "INSERT INTO rtt_pathways (id, person_id, target_service, started_at, created_at, updated_at) \
         SELECT gen_random_uuid(), gen_random_uuid(), $1, \
                now() - make_interval(years => $3::int), now(), now() \
         FROM generate_series(1, $2::int)",
        [service.to_string().into(), count.into(), years.into()],
    ))
    .await
    .expect("add pathways");
}

async fn remove_pathways(s: &AppState, service: &str) {
    s.db.run(Stmt::new(
        "DELETE FROM rtt_clock_events WHERE pathway_id IN \
           (SELECT id FROM rtt_pathways WHERE target_service = $1)",
        [service.to_string().into()],
    ))
    .await
    .expect("remove events");
    s.db.run(Stmt::new(
        "DELETE FROM rtt_pathways WHERE target_service = $1",
        [service.to_string().into()],
    ))
    .await
    .expect("remove pathways");
}

/// The report once read 20,000 open pathways, in no order, so a board's figures came from an arbitrary part of
/// its list.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_report_counts_every_open_pathway_past_twenty_thousand() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("qual48-{}", Uuid::new_v4());
    add_pathways(&s, &service, 20_001, 1).await;

    let report = s
        .reports
        .clinic_activity(
            Some(Utc::now() - Duration::days(1)),
            Some(Utc::now()),
            GroupBy::Day,
        )
        .await;
    remove_pathways(&s, &service).await;

    let report = report.unwrap();
    let mine = report
        .pathway_weeks
        .iter()
        .filter(|(name, _)| *name == service)
        .count();
    assert_eq!(mine, 20_001);
}

/// The list once stopped at the 5,000 oldest before a campaign filtered it by service, so a service newer than
/// those had an empty cohort however long its patients had waited.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_long_waiter_behind_five_thousand_older_pathways_is_still_due() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let older = format!("qual48-{}", Uuid::new_v4());
    let service = format!("qual48-{}", Uuid::new_v4());
    add_pathways(&s, &older, 5_001, 3).await;
    add_pathways(&s, &service, 1, 2).await;
    s.db.run(Stmt::new(
        "INSERT INTO rtt_clock_events (id, pathway_id, kind, event_at, created_at) \
         SELECT gen_random_uuid(), id, 'started', started_at, now() FROM rtt_pathways \
         WHERE target_service = $1",
        [service.clone().into()],
    ))
    .await
    .expect("start the clock");

    let due = s.waits.due_for_check(Some(26), Some(90)).await;
    remove_pathways(&s, &older).await;
    remove_pathways(&s, &service).await;

    let due = due.unwrap();
    assert!(due.iter().any(|(_, p)| p.target_service == service));
}

/// An audit that read only part of its period says so; one that read all of it does not. Seeds its own two
/// visits, so a fresh database does not pass with nothing to cut.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_fairness_audit_cut_short_says_so() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    for days in [1, 2] {
        let start = Utc::now() - Duration::days(days);
        let visit = Appointment::new(Uuid::new_v4(), start, start + Duration::minutes(15));
        let visit = AppointmentRepository::create(&s.db, &visit).await.unwrap();
        s.db.run(Stmt::new(
            "UPDATE appointments SET status = 'fulfilled' WHERE id = $1",
            [visit.id.into()],
        ))
        .await
        .expect("a past visit");
    }

    let whole = s.noshow.fairness_audit(1825, Some(1)).await.unwrap();
    assert!(!whole.truncated);
    let read = whole.overall.expect("two visits at least").appointments;
    assert!(read >= 2);

    let exact = s
        .noshow
        .fairness_audit_of(1825, Some(1), read)
        .await
        .unwrap();
    assert!(!exact.truncated, "exactly the limit is not cut");

    let cut = s
        .noshow
        .fairness_audit_of(1825, Some(1), read - 1)
        .await
        .unwrap();
    assert!(cut.truncated);
    assert_eq!(cut.overall.unwrap().appointments, read - 1);
}
