//! QUAL-32: NFR-1 and NFR-2 as properties, over HTTP. Needs PostgreSQL: `cargo test -- --ignored --test-threads=1`.
//!
//! - Every mutation in the table below writes exactly one audit row (NFR-1), and the scheduling ones also
//!   one outbox event (NFR-2).
//! - A change, its audit row and its outbox event are one transaction: when the outbox insert is made to
//!   fail, the request fails, the appointment is unchanged and no audit row is left behind.
//!
//! Counting rows before and after a request assumes nothing else writes to this database meanwhile, which
//! `--test-threads=1` on a test database gives. The table covers the core mutations, not every route; which
//! state changes publish an event at all is an open question (see `tasks.md`, QUAL-32).

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

async fn count(s: &AppState, table: &str) -> i64 {
    s.db.query_one_raw(Statement::from_string(
        DbBackend::Postgres,
        format!("SELECT count(*) AS n FROM {table}"),
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<i64>("", "n")
    .unwrap()
}

async fn send(s: &AppState, method: &str, uri: &str, body: serde_json::Value) -> StatusCode {
    let app = patient_administration_system::api::rest::router(s.clone());
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

/// The change in rows a request made: (status, audit rows, outbox events).
async fn delta(
    s: &AppState,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, i64, i64) {
    let (a, o) = (count(s, "audit_log").await, count(s, "outbox_events").await);
    let status = send(s, method, uri, body).await;
    (
        status,
        count(s, "audit_log").await - a,
        count(s, "outbox_events").await - o,
    )
}

async fn booked(s: &AppState, schedule: Uuid, days: i64) -> Uuid {
    let slot = common::free_slot(&s.db, schedule, days).await;
    s.scheduling
        .book_slot(slot.id, Uuid::new_v4(), &UserContext::system())
        .await
        .unwrap()
        .id
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn each_core_mutation_writes_one_audit_row_and_the_scheduling_ones_one_event() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    let person = Uuid::new_v4();

    let free = common::free_slot(&s.db, schedule, 3).await.id;
    let (a1, a2, a3, a4) = (
        booked(&s, schedule, 4).await,
        booked(&s, schedule, 5).await,
        booked(&s, schedule, 6).await,
        booked(&s, schedule, 7).await,
    );
    // (what, method, path, body, events expected)
    let cases: Vec<(&str, &str, String, serde_json::Value, bool)> = vec![
        (
            "book a slot",
            "POST",
            format!("/api/slots/{free}/book"),
            serde_json::json!({ "person_id": person }),
            true,
        ),
        (
            "cancel",
            "POST",
            format!("/api/appointments/{a1}/cancel"),
            serde_json::json!({ "barrier_code": "work" }),
            true,
        ),
        (
            "check in",
            "POST",
            format!("/api/appointments/{a2}/check-in"),
            serde_json::json!({}),
            true,
        ),
        (
            "no-show",
            "POST",
            format!("/api/appointments/{a3}/no-show"),
            serde_json::json!({}),
            true,
        ),
        (
            "check in, then complete",
            "POST",
            format!("/api/appointments/{a4}/check-in"),
            serde_json::json!({}),
            true,
        ),
        (
            "complete",
            "POST",
            format!("/api/appointments/{a4}/complete"),
            serde_json::json!({}),
            true,
        ),
        (
            "join the waiting list",
            "POST",
            "/api/waitlist".into(),
            serde_json::json!({ "person_id": person, "target_service": service, "priority": "routine" }),
            true,
        ),
        (
            "join standby",
            "POST",
            "/api/standby".into(),
            serde_json::json!({ "person_id": person, "service_type": service }),
            false,
        ),
        (
            "receive a referral",
            "POST",
            "/api/referrals".into(),
            serde_json::json!({ "person_id": person, "source": "gp", "target_service": service }),
            false,
        ),
        (
            "raise a support flag",
            "POST",
            "/api/support-flags".into(),
            serde_json::json!({ "person_id": person, "kind": "needs_support_call", "created_by": "Nurse A" }),
            false,
        ),
    ];
    let mut wrong = Vec::new();
    for (what, method, uri, body, events) in cases {
        let (status, audits, outbox) = delta(&s, method, &uri, body).await;
        if !status.is_success() {
            wrong.push(format!("{what}: {status}"));
            continue;
        }
        if audits != 1 {
            wrong.push(format!("{what}: {audits} audit rows, not 1 (NFR-1)"));
        }
        if events && outbox < 1 {
            wrong.push(format!("{what}: no outbox event (NFR-2)"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_change_its_audit_row_and_its_event_commit_together_or_not_at_all() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await.id;
    let a = booked(&s, schedule, 5).await;
    let audits = count(&s, "audit_log").await;

    // Make every new outbox insert fail, cancel, and put things back before asserting anything.
    s.db.execute_unprepared(
        "ALTER TABLE outbox_events ADD CONSTRAINT qual32_refuse_events CHECK (false) NOT VALID",
    )
    .await
    .unwrap();
    let status = send(
        &s,
        "POST",
        &format!("/api/appointments/{a}/cancel"),
        serde_json::json!({}),
    )
    .await;
    s.db.execute_unprepared("ALTER TABLE outbox_events DROP CONSTRAINT qual32_refuse_events")
        .await
        .unwrap();

    assert!(
        status.is_server_error(),
        "the cancel must fail when its event cannot be written: {status}"
    );
    let row =
        s.db.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT status FROM appointments WHERE id = $1",
            [a.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "status").unwrap(),
        "booked",
        "the change was rolled back"
    );
    assert_eq!(
        count(&s, "audit_log").await,
        audits,
        "and so was its audit row"
    );

    // With events allowed again the same cancel goes through.
    assert_eq!(
        send(
            &s,
            "POST",
            &format!("/api/appointments/{a}/cancel"),
            serde_json::json!({})
        )
        .await,
        StatusCode::OK
    );
}
