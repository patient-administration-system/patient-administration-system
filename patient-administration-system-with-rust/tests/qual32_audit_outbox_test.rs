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

/// The two routers `main` serves together.
fn app(s: &AppState) -> axum::Router {
    patient_administration_system::api::rest::router(s.clone()).merge(
        patient_administration_system::api::fhir::fhir_router(s.clone()),
    )
}

async fn send(s: &AppState, method: &str, uri: &str, body: serde_json::Value) -> StatusCode {
    app(s)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("if-match", "*")
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

fn fhir_schedule_body() -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Schedule",
        "actor": [{ "reference": format!("Practitioner/{}", Uuid::new_v4()) }],
    })
}

fn fhir_slot_body(schedule: Uuid) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Slot",
        "schedule": { "reference": format!("Schedule/{schedule}") },
        "status": "free",
        "start": "2031-03-04T09:00:00Z",
        "end": "2031-03-04T09:30:00Z",
    })
}

/// A request as a named user, and the id in its answer when it has one.
async fn send_as(
    s: &AppState,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, Option<Uuid>) {
    let answer = app(s)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("if-match", "*")
                .header("x-user-id", "fhir2-tester")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = answer.status();
    let bytes = axum::body::to_bytes(answer.into_body(), usize::MAX)
        .await
        .unwrap();
    let id = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("id")?.as_str()?.parse().ok());
    (status, id)
}

/// The audit rows of one record, oldest first: (action, old value?, new value?, who).
async fn audit_of(s: &AppState, entity: &str, id: Uuid) -> Vec<(String, bool, bool, String)> {
    s.db.query_all_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT action, old_value IS NOT NULL AS had_old, new_value IS NOT NULL AS had_new, \
         user_id FROM audit_log WHERE entity_type = $1 AND entity_id = $2 ORDER BY at",
        [entity.into(), id.into()],
    ))
    .await
    .unwrap()
    .iter()
    .map(|row| {
        (
            row.try_get::<String>("", "action").unwrap(),
            row.try_get::<bool>("", "had_old").unwrap(),
            row.try_get::<bool>("", "had_new").unwrap(),
            row.try_get::<String>("", "user_id").unwrap(),
        )
    })
    .collect()
}

/// FR-204: each direct write says what it did, and who did it.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_direct_fhir_write_records_its_action_its_values_its_user_and_its_event() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let who = "fhir2-tester".to_string();
    let lifecycle =
        |action: &str, had_old, had_new| (action.to_string(), had_old, had_new, who.clone());
    let expected = vec![
        lifecycle("create", false, true),
        lifecycle("update", true, true),
        lifecycle("delete", true, false),
    ];

    let (status, schedule) = send_as(&s, "POST", "/fhir/Schedule", fhir_schedule_body()).await;
    assert!(status.is_success(), "{status}");
    let schedule = schedule.expect("the created Schedule has an id");
    let uri = format!("/fhir/Schedule/{schedule}");
    let update = serde_json::json!({ "active": false });
    assert!(
        send_as(&s, "PUT", &uri, update.clone())
            .await
            .0
            .is_success()
    );

    let (status, slot) = send_as(&s, "POST", "/fhir/Slot", fhir_slot_body(schedule)).await;
    assert!(status.is_success(), "{status}");
    let slot = slot.expect("the created Slot has an id");
    let slot_uri = format!("/fhir/Slot/{slot}");
    let change = serde_json::json!({ "status": "busy-unavailable" });
    assert!(send_as(&s, "PUT", &slot_uri, change).await.0.is_success());
    assert_eq!(
        send_as(&s, "DELETE", &slot_uri, serde_json::json!({}))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send_as(&s, "DELETE", &uri, serde_json::json!({})).await.0,
        StatusCode::NO_CONTENT
    );

    assert_eq!(audit_of(&s, "schedule", schedule).await, expected);
    assert_eq!(audit_of(&s, "slot", slot).await, expected);
    for (event, id) in [
        ("ScheduleCreated", schedule),
        ("ScheduleUpdated", schedule),
        ("ScheduleDeleted", schedule),
        ("SlotCreated", slot),
        ("SlotUpdated", slot),
        ("SlotDeleted", slot),
    ] {
        assert_eq!(
            count(
                &s,
                &format!("outbox_events WHERE event_type = '{event}' AND payload->>'id' = '{id}'"),
            )
            .await,
            1,
            "{event}"
        );
    }
}

/// FHIR-4: a `PUT` meets the rule a `POST` does, whichever end it moves.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_slot_put_cannot_end_a_slot_before_it_starts() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 5).await;
    let uri = format!("/fhir/Slot/{}", slot.id);
    let audits = count(&s, "audit_log").await;
    // By an hour: the stored times are rounded to microseconds, so the Slot's own instants may not match them.
    let hour = chrono::Duration::hours(1);
    for body in [
        serde_json::json!({ "end": (slot.start_datetime - hour).to_rfc3339() }),
        serde_json::json!({ "start": (slot.end_datetime + hour).to_rfc3339() }),
    ] {
        assert_eq!(send(&s, "PUT", &uri, body).await, StatusCode::BAD_REQUEST);
    }
    assert_eq!(count(&s, "audit_log").await, audits, "nothing was written");
    assert_eq!(
        send(&s, "PUT", &uri, serde_json::json!({ "status": "busy" })).await,
        StatusCode::OK,
        "a PUT that moves neither end is not affected"
    );
}

/// FR-204: a refused or failed write leaves nothing behind.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_direct_fhir_write_commits_with_its_audit_row_and_event_or_not_at_all() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 5).await.id;
    let slot_uri = format!("/fhir/Slot/{slot}");
    let schedule_uri = format!("/fhir/Schedule/{schedule}");
    let empty = serde_json::json!({});
    let (audits, schedules, slots) = (
        count(&s, "audit_log").await,
        count(&s, "schedules").await,
        count(&s, "slots").await,
    );

    s.db.execute_unprepared(
        "ALTER TABLE outbox_events ADD CONSTRAINT fhir2_refuse_events CHECK (false) NOT VALID",
    )
    .await
    .unwrap();
    let attempts = [
        send(&s, "POST", "/fhir/Schedule", fhir_schedule_body()).await,
        send(
            &s,
            "PUT",
            &schedule_uri,
            serde_json::json!({ "active": false }),
        )
        .await,
        send(&s, "POST", "/fhir/Slot", fhir_slot_body(schedule)).await,
        send(
            &s,
            "PUT",
            &slot_uri,
            serde_json::json!({ "status": "busy" }),
        )
        .await,
        send(&s, "DELETE", &slot_uri, empty.clone()).await,
        send(&s, "DELETE", &schedule_uri, empty.clone()).await,
    ];
    s.db.execute_unprepared("ALTER TABLE outbox_events DROP CONSTRAINT fhir2_refuse_events")
        .await
        .unwrap();

    for status in attempts {
        assert!(
            status.is_server_error(),
            "a write must fail with its event: {status}"
        );
    }
    assert_eq!(count(&s, "audit_log").await, audits, "no audit row is left");
    assert_eq!(count(&s, "schedules").await, schedules, "nor a Schedule");
    assert_eq!(
        count(&s, "slots").await,
        slots,
        "nor a Slot, and none is gone"
    );
    let unchanged = count(
        &s,
        &format!(
            "slots WHERE id = '{slot}' AND status = 'free' \
             AND schedule_id IN (SELECT id FROM schedules WHERE id = '{schedule}' AND active)"
        ),
    )
    .await;
    assert_eq!(unchanged, 1, "the updates were undone");

    // A Slot naming no Schedule is the caller's mistake, not a server error.
    let (status, audits) = {
        let audits = count(&s, "audit_log").await;
        (
            send(&s, "POST", "/fhir/Slot", fhir_slot_body(Uuid::new_v4())).await,
            audits,
        )
    };
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(count(&s, "audit_log").await, audits);

    // A record that is not there is a 404, and is not audited.
    let missing = Uuid::new_v4();
    for (method, uri) in [
        ("PUT", format!("/fhir/Slot/{missing}")),
        ("DELETE", format!("/fhir/Slot/{missing}")),
        ("PUT", format!("/fhir/Schedule/{missing}")),
        ("DELETE", format!("/fhir/Schedule/{missing}")),
    ] {
        assert_eq!(
            send(&s, method, &uri, empty.clone()).await,
            StatusCode::NOT_FOUND,
            "{method} {uri}"
        );
    }
    assert_eq!(count(&s, "audit_log").await, audits);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn each_core_mutation_writes_one_audit_row_and_the_scheduling_ones_one_event() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    let person = Uuid::new_v4();

    let free = common::free_slot(&s.db, schedule, 3).await.id;
    let (fhir_slot, fhir_gone_slot) = (
        common::free_slot(&s.db, schedule, 8).await.id,
        common::free_slot(&s.db, schedule, 9).await.id,
    );
    let (fhir_schedule, fhir_gone_schedule) = (
        common::new_schedule(&s.db).await.id,
        common::new_schedule(&s.db).await.id,
    );
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
        (
            "create a FHIR Schedule",
            "POST",
            "/fhir/Schedule".into(),
            fhir_schedule_body(),
            true,
        ),
        (
            "update a FHIR Schedule",
            "PUT",
            format!("/fhir/Schedule/{fhir_schedule}"),
            serde_json::json!({ "active": false }),
            true,
        ),
        (
            "delete a FHIR Schedule",
            "DELETE",
            format!("/fhir/Schedule/{fhir_gone_schedule}"),
            serde_json::json!({}),
            true,
        ),
        (
            "create a FHIR Slot",
            "POST",
            "/fhir/Slot".into(),
            fhir_slot_body(schedule),
            true,
        ),
        (
            "update a FHIR Slot",
            "PUT",
            format!("/fhir/Slot/{fhir_slot}"),
            serde_json::json!({ "status": "busy-unavailable" }),
            true,
        ),
        (
            "delete a FHIR Slot",
            "DELETE",
            format!("/fhir/Slot/{fhir_gone_slot}"),
            serde_json::json!({}),
            true,
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
