//! CAP-8: wait-time and lead-time data. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Tests that start an RTT pathway remove it again: the RTT cockpit lists every open pathway and looks up
//! each patient, and these patients are not in the patients table.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::waitlist::WaitlistRepository;
use patient_administration_system::models::waitlist::{Priority, WaitlistStatus};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

/// A pathway whose clock started `weeks` weeks ago.
async fn pathway(s: &AppState, person: Uuid, weeks: i64) -> Uuid {
    let p = s
        .rtt
        .start(person, format!("svc-{}", Uuid::new_v4()), &sys())
        .await
        .unwrap();
    s.db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE rtt_clock_events SET event_at = now() - make_interval(weeks => $2::int) WHERE pathway_id = $1",
        [p.id.into(), (weeks as i32).into()],
    ))
    .await
    .unwrap();
    p.id
}

async fn remove(s: &AppState, pathway: Uuid) {
    for sql in [
        "DELETE FROM still_needed_checks WHERE pathway_id = $1",
        "DELETE FROM rtt_clock_events WHERE pathway_id = $1",
        "DELETE FROM rtt_pathways WHERE id = $1",
    ] {
        s.db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [pathway.into()],
        ))
        .await
        .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn lead_time_is_computed_from_the_booking_time() {
    let s = state().await;
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 30).await;
    let person = Uuid::new_v4();
    let a = s
        .scheduling
        .book_slot(slot.id, person, &sys())
        .await
        .unwrap();
    // Booked 21 days before the visit.
    s.db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE appointments SET created_at = start_datetime - interval '21 days 2 hours' WHERE id = $1",
        [a.id.into()],
    ))
    .await
    .unwrap();
    let lt = s.waits.lead_time(a.id).await.unwrap();
    assert_eq!(lt.lead_days, 21);
    assert_eq!(lt.starts_at, a.start_datetime);
    let w = s.waits.patient_waits(person).await.unwrap();
    assert_eq!(w.appointments.len(), 1);
    assert_eq!(w.appointments[0].lead_days, 21);
    assert!(s.waits.lead_time(Uuid::new_v4()).await.is_err());

    // A cancelled appointment is not a wait the patient is still having.
    s.scheduling
        .cancel(
            a.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &sys(),
        )
        .await
        .unwrap();
    assert!(
        s.waits
            .patient_waits(person)
            .await
            .unwrap()
            .appointments
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_long_wait_with_no_recent_check_is_listed_and_a_check_takes_it_off() {
    let s = state().await;
    let (long, short, checked) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let a = pathway(&s, long, 30).await;
    let b = pathway(&s, short, 10).await;
    let c = pathway(&s, checked, 40).await;
    s.waits.ask(c, "Nurse B", &sys()).await.unwrap();

    let ids = |v: Vec<(Uuid, patient_administration_system::waits::PathwayWait)>| {
        v.into_iter().map(|(_, p)| p.pathway_id).collect::<Vec<_>>()
    };
    let due = ids(s.waits.due_for_check(Some(26), Some(90)).await.unwrap());
    assert!(due.contains(&a), "over 26 weeks, never asked");
    assert!(!due.contains(&b), "only 10 weeks");
    assert!(!due.contains(&c), "asked recently");
    // The threshold is a parameter.
    assert!(ids(s.waits.due_for_check(Some(5), Some(90)).await.unwrap()).contains(&b));

    s.waits.ask(a, "Nurse B", &sys()).await.unwrap();
    assert!(!ids(s.waits.due_for_check(Some(26), Some(90)).await.unwrap()).contains(&a));

    // The wait shown for the patient is the clock's figure.
    let w = s.waits.patient_waits(long).await.unwrap();
    assert!(
        (29..=31).contains(&w.pathways[0].weeks_waited),
        "{}",
        w.pathways[0].weeks_waited
    );
    assert!(w.pathways[0].last_check_at.is_some());
    for p in [a, b, c] {
        remove(&s, p).await;
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn no_longer_needed_is_recorded_and_audited_and_removes_nobody() {
    let s = state().await;
    let person = Uuid::new_v4();
    let service = format!("svc-{}", Uuid::new_v4());
    let entry = s
        .waitlist
        .add(None, person, service.clone(), Priority::Routine, &sys())
        .await
        .unwrap();
    let p = s.rtt.start(person, service, &sys()).await.unwrap();
    let count = |id: Uuid| {
        let db = s.db.clone();
        async move {
            db.query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM rtt_clock_events WHERE pathway_id = $1",
                [id.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "n")
            .unwrap()
        }
    };
    let events = count(p.id).await;

    let check = s.waits.ask(p.id, "Nurse B", &sys()).await.unwrap();
    s.waits
        .answer(check, "no_longer_needed", "Nurse B", &sys())
        .await
        .unwrap();
    assert_eq!(
        common::audit_count(&s.db, p.id, "still_needed_asked").await,
        1
    );
    assert_eq!(
        common::audit_count(&s.db, p.id, "still_needed_answered").await,
        1
    );

    // Recorded, and nothing else moved: still on the list, clock untouched, pathway still open.
    let e = WaitlistRepository::find_entry_by_id(&s.db, entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e.status, WaitlistStatus::Waiting);
    assert_eq!(count(p.id).await, events);
    let open =
        s.db.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT stopped_at IS NULL AS open FROM rtt_pathways WHERE id = $1",
            [p.id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert!(open.try_get::<bool>("", "open").unwrap());
    remove(&s, p.id).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_check_needs_names_and_a_valid_answer_and_is_answered_once() {
    let s = state().await;
    let p = pathway(&s, Uuid::new_v4(), 1).await;
    assert!(s.waits.ask(p, " ", &sys()).await.is_err());
    assert!(
        s.waits.ask(Uuid::new_v4(), "A", &sys()).await.is_err(),
        "no such pathway"
    );
    let c = s.waits.ask(p, "Nurse B", &sys()).await.unwrap();
    assert!(s.waits.answer(c, "maybe", "Nurse B", &sys()).await.is_err());
    assert!(
        s.waits
            .answer(c, "still_needed", "  ", &sys())
            .await
            .is_err()
    );
    s.waits
        .answer(c, "still_needed", "Nurse B", &sys())
        .await
        .unwrap();
    assert!(
        s.waits
            .answer(c, "unsure", "Nurse B", &sys())
            .await
            .is_err(),
        "once"
    );
    remove(&s, p).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_work() {
    let s = state().await;
    let person = Uuid::new_v4();
    let p = pathway(&s, person, 30).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let call = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let mut b = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    b = b.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(b.body(body).unwrap()).await.unwrap();
            let st = r.status();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(
                    &to_bytes(r.into_body(), 1 << 20).await.unwrap(),
                )
                .unwrap_or_default(),
            )
        }
    };
    let (st, v) = call(
        "GET",
        "/api/pathways/due-check?weeks=26&repeat_days=90".into(),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["pathway_id"] == p.to_string() && x["person_id"] == person.to_string())
    );
    let (st, _) = call(
        "POST",
        format!("/api/pathways/{p}/still-needed-checks"),
        Some(serde_json::json!({ "asked_by": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        "POST",
        format!("/api/pathways/{p}/still-needed-checks"),
        Some(serde_json::json!({ "asked_by": "Nurse B" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let check = v["data"]["id"].as_str().unwrap().to_string();
    let (st, _) = call(
        "POST",
        format!("/api/still-needed-checks/{check}/answer"),
        Some(serde_json::json!({ "answer": "unsure", "recorded_by": "Nurse B" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        "POST",
        format!("/api/still-needed-checks/{check}/answer"),
        Some(serde_json::json!({ "answer": "unsure", "recorded_by": "Nurse B" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, v) = call("GET", format!("/api/patients/{person}/waits"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"]["pathways"][0]["pathway_id"], p.to_string());
    let (st, _) = call(
        "GET",
        format!("/api/appointments/{}/lead-time", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    remove(&s, p).await;
}
