//! CAP-15: non-emergency transport booked from a confirmed appointment.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jiff::SignedDuration;
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::transport::{
    TransportClient, TransportConfirmation, TransportRequest,
};
use patient_administration_system::upstream::{Person, StubPersonClient};
use patient_administration_system::writeback::{ReplyAction, ReplyRequest};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

#[derive(Default)]
struct Recording {
    booked: Mutex<Vec<serde_json::Value>>,
    cancelled: Mutex<Vec<String>>,
    fail_book: Mutex<bool>,
    fail_cancel: Mutex<bool>,
}

#[async_trait]
impl TransportClient for Recording {
    async fn book(
        &self,
        req: &TransportRequest,
    ) -> patient_administration_system::Result<TransportConfirmation> {
        if *self.fail_book.lock().unwrap() {
            return Err(patient_administration_system::Error::upstream(format!(
                "provider down for {}",
                req.pickup_address
            )));
        }
        self.booked
            .lock()
            .unwrap()
            .push(serde_json::to_value(req).unwrap());
        Ok(TransportConfirmation {
            provider_ref: format!("prov-{}", req.reference),
            window_start: req.appointment_start - SignedDuration::from_mins(80),
            window_end: req.appointment_start - SignedDuration::from_mins(50),
        })
    }
    async fn cancel(&self, provider_ref: &str) -> patient_administration_system::Result<()> {
        if *self.fail_cancel.lock().unwrap() {
            return Err(patient_administration_system::Error::upstream("nope"));
        }
        self.cancelled
            .lock()
            .unwrap()
            .push(provider_ref.to_string());
        Ok(())
    }
}

struct W {
    s: AppState,
    rec: Arc<Recording>,
    person: Uuid,
    schedule: Uuid,
}

async fn world(with_address: bool) -> W {
    world_with(with_address, true).await
}

/// `provider: false` is the server's ordinary wiring with no transport provider configured (PA-01).
async fn world_with(with_address: bool, provider: bool) -> W {
    let rec = Arc::new(Recording::default());
    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        serde_json::json!({ "given": "Ada", "family": "Lovelace" }),
    );
    if with_address {
        data.insert(
            "addresses".into(),
            serde_json::json!([{ "line": ["12 Mill Lane", "Flat 3"], "city": "Cardiff", "postal_code": "CF10 1AA" }]),
        );
    }
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub);
    assert!(
        !s.transport.has_provider(),
        "no provider unless one is configured"
    );
    let s = if provider {
        s.with_transport_client(rec.clone())
    } else {
        s
    };
    let schedule = common::new_schedule(&s.db).await.id;
    W {
        s,
        rec,
        person,
        schedule,
    }
}

impl W {
    async fn book(&self) -> patient_administration_system::models::appointment::Appointment {
        let slot = common::free_slot(&self.s.db, self.schedule, 4).await;
        self.s
            .scheduling
            .book_slot(slot.id, self.person, &sys())
            .await
            .unwrap()
    }
    async fn entitle(&self) {
        self.s
            .transport
            .set_eligible(self.person, "Dr A. Clinician", "NEPTS criteria met", &sys())
            .await
            .unwrap();
        self.s
            .transport
            .record_consent(self.person, "Nurse B", "Art. 6(1)(e) and 9(2)(h)", &sys())
            .await
            .unwrap();
    }
    async fn confirm(&self, a: Uuid) {
        let r = self
            .s
            .writeback
            .apply(
                ReplyRequest {
                    message_id: format!("m-{}", Uuid::new_v4()),
                    action: ReplyAction::Confirm,
                    appointment_id: a,
                    person_id: self.person,
                    channel: Some("sms".into()),
                    barrier_code: None,
                    new_slot_id: None,
                    modality: None,
                    received_at: None,
                },
                &sys(),
            )
            .await
            .unwrap();
        assert_eq!(r.reject_reason, None);
    }
    fn booked(&self) -> usize {
        self.rec.booked.lock().unwrap().len()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn confirming_for_an_eligible_consenting_patient_books_exactly_one_ride_with_a_minimal_payload()
 {
    let w = world(true).await;
    w.entitle().await;
    let a = w.book().await;
    assert_eq!(w.booked(), 0, "nothing before the appointment is confirmed");
    assert!(w.s.transport.booking_for(a.id).await.unwrap().is_none());

    w.confirm(a.id).await;
    assert_eq!(w.booked(), 1);
    let b = w.s.transport.booking_for(a.id).await.unwrap().unwrap();
    assert_eq!(b.status, "booked");
    assert!(
        b.window_start.unwrap() < b.window_end.unwrap(),
        "the pick-up window is recorded"
    );
    assert_eq!(
        common::audit_count(&w.s.db, a.id, "transport_booked").await,
        1
    );

    // Confirming again (another message) does not book a second ride.
    w.confirm(a.id).await;
    assert_eq!(w.booked(), 1);

    // The provider is told the address and the times, and nothing else.
    let req = w.rec.booked.lock().unwrap()[0].clone();
    let mut keys: Vec<&str> = req
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "appointment_end",
            "appointment_start",
            "destination",
            "pickup_address",
            "reference"
        ]
    );
    assert_eq!(
        req["pickup_address"],
        "12 Mill Lane, Flat 3, Cardiff, CF10 1AA"
    );
    let text = req.to_string();
    for secret in [
        w.person.to_string(),
        a.id.to_string(),
        "Ada".into(),
        "Lovelace".into(),
    ] {
        assert!(
            !text.contains(&secret),
            "the payload must not carry {secret}"
        );
    }
    // And the PAS did not keep the address.
    let stored: i64 = {
        w.s.db
            .one(Stmt::text(
                "SELECT count(*) AS n FROM transport_bookings WHERE last_error LIKE '%Mill Lane%'"
                    .to_string(),
            ))
            .await
            .unwrap()
            .unwrap()
            .col("n")
            .unwrap()
    };
    assert_eq!(stored, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn no_eligibility_or_no_consent_or_withdrawn_consent_books_nothing() {
    // Consent but no eligibility.
    let w = world(true).await;
    w.s.transport
        .record_consent(w.person, "Nurse B", "basis", &sys())
        .await
        .unwrap();
    let a = w.book().await;
    w.confirm(a.id).await;
    assert_eq!(w.booked(), 0, "not eligible");

    // Eligibility but no consent.
    let w = world(true).await;
    w.s.transport
        .set_eligible(w.person, "Dr A", "basis", &sys())
        .await
        .unwrap();
    let a = w.book().await;
    w.confirm(a.id).await;
    assert_eq!(w.booked(), 0, "no consent to share the address");

    // Both, then consent withdrawn before confirmation.
    let w = world(true).await;
    w.entitle().await;
    let a = w.book().await;
    w.s.transport
        .withdraw_consent(w.person, &sys())
        .await
        .unwrap();
    w.confirm(a.id).await;
    assert_eq!(w.booked(), 0, "consent withdrawn");
    assert_eq!(
        common::audit_count(&w.s.db, w.person, "transport_consent_withdrawn").await,
        1
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn cancelling_or_moving_the_appointment_cancels_the_ride() {
    let w = world(true).await;
    w.entitle().await;
    let a = w.book().await;
    w.confirm(a.id).await;
    w.s.scheduling
        .cancel(
            a.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(w.rec.cancelled.lock().unwrap().len(), 1);
    assert_eq!(
        w.s.transport
            .booking_for(a.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "cancelled"
    );

    // Moving is a cancel of the old appointment: its ride goes, and the new one gets a ride on confirmation.
    let w = world(true).await;
    w.entitle().await;
    let a = w.book().await;
    w.confirm(a.id).await;
    let to = common::free_slot(&w.s.db, w.schedule, 6).await;
    let (_, new) = w.s.scheduling.rebook(a.id, to.id, &sys()).await.unwrap();
    assert_eq!(
        w.rec.cancelled.lock().unwrap().len(),
        1,
        "the old ride is cancelled"
    );
    assert!(
        w.s.transport.booking_for(new.id).await.unwrap().is_none(),
        "not before confirmation"
    );
    w.confirm(new.id).await;
    assert_eq!(w.booked(), 2, "a new ride for the new appointment");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_provider_failure_keeps_the_appointment_and_puts_the_patient_on_the_call_list() {
    let w = world(true).await;
    w.entitle().await;
    *w.rec.fail_book.lock().unwrap() = true;
    let a = w.book().await;
    w.confirm(a.id).await;
    let now = AppointmentRepository::find_by_id(&w.s.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert!(now.confirmed_at.is_some(), "still confirmed");
    assert_eq!(
        now.status,
        patient_administration_system::models::appointment::AppointmentStatus::Booked
    );
    let b = w.s.transport.booking_for(a.id).await.unwrap().unwrap();
    assert_eq!(b.status, "failed");
    assert!(
        !b.last_error.unwrap().contains("Mill Lane"),
        "the address is not kept in an error"
    );
    assert!(common::call_is_open(&w.s.db, a.id, "transport_failed").await);

    // A failed cancel is also put to a person.
    let w = world(true).await;
    w.entitle().await;
    let a = w.book().await;
    w.confirm(a.id).await;
    *w.rec.fail_cancel.lock().unwrap() = true;
    w.s.scheduling
        .cancel(
            a.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &sys(),
        )
        .await
        .unwrap();
    assert!(common::call_is_open(&w.s.db, a.id, "transport_failed").await);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn no_address_on_record_is_a_failure_for_a_person_not_a_guess() {
    let w = world(false).await;
    w.entitle().await;
    let a = w.book().await;
    w.confirm(a.id).await;
    assert_eq!(w.booked(), 0);
    let b = w.s.transport.booking_for(a.id).await.unwrap().unwrap();
    assert_eq!(b.status, "failed");
    assert!(b.last_error.unwrap().contains("no pick-up address"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_record_decisions_with_names_and_show_the_ride() {
    let w = world(true).await;
    let app = patient_administration_system::api::rest::router(w.s.clone());
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
    let p = w.person;
    let (st, _) = call(
        "PUT",
        format!("/api/patients/{p}/transport-eligibility"),
        Some(serde_json::json!({ "eligible_by": " ", "basis": "x" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        "PUT",
        format!("/api/patients/{p}/transport-eligibility"),
        Some(serde_json::json!({ "eligible_by": "Dr A", "basis": "NEPTS" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        "PUT",
        format!("/api/patients/{p}/transport-consent"),
        Some(serde_json::json!({ "recorded_by": "Nurse B", "basis": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "the lawful basis is required");
    let (st, _) = call(
        "PUT",
        format!("/api/patients/{p}/transport-consent"),
        Some(serde_json::json!({ "recorded_by": "Nurse B", "basis": "Art. 6(1)(e)" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let a = w.book().await;
    w.confirm(a.id).await;
    let (st, v) = call("GET", format!("/api/appointments/{}/transport", a.id), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"]["status"], "booked");
    assert!(v["data"]["window_start"].is_string());

    let (st, _) = call(
        "DELETE",
        format!("/api/patients/{p}/transport-consent"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        "DELETE",
        format!("/api/patients/{p}/transport-consent"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// The rides nobody booked, read through the API: every page, as a client would.
async fn not_booked(s: &AppState) -> Vec<serde_json::Value> {
    let app = patient_administration_system::api::rest::router(s.clone());
    let mut all = Vec::new();
    let mut uri = Some("/api/transport/not-booked?limit=500".to_string());
    while let Some(u) = uri.take() {
        let r = app
            .clone()
            .oneshot(Request::get(&u).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let v: serde_json::Value =
            serde_json::from_slice(&to_bytes(r.into_body(), 1 << 24).await.unwrap()).unwrap();
        all.extend(v["data"].as_array().unwrap().iter().cloned());
        uri = v["meta"]["next"].as_str().map(String::from);
    }
    all
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn with_no_provider_a_ride_is_not_booked_and_staff_see_it() {
    // No address on record: reading one would fail the booking, so `no_provider` also shows none was read.
    let w = world_with(false, false).await;
    w.entitle().await;
    let a = w.book().await;
    w.confirm(a.id).await;
    w.confirm(a.id).await;

    let b = w.s.transport.booking_for(a.id).await.unwrap().unwrap();
    assert_eq!(b.status, "no_provider");
    assert_eq!(b.provider_ref, None, "no made-up reference");
    assert_eq!(
        (b.window_start, b.window_end),
        (None, None),
        "no made-up pick-up window"
    );
    let rows: i64 =
        w.s.db
            .one(Stmt::new(
                "SELECT count(*)::bigint AS n FROM transport_bookings WHERE appointment_id = $1",
                [a.id.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .col("n")
            .unwrap();
    assert_eq!(rows, 1, "confirming twice records it once");
    assert_eq!(
        common::audit_count(&w.s.db, a.id, "transport_not_booked").await,
        1
    );
    assert_eq!(
        common::audit_count(&w.s.db, a.id, "transport_booked").await,
        0
    );
    // The appointment is untouched.
    let still = AppointmentRepository::find_by_id(&w.s.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, a.status);
    assert!(still.confirmed_at.is_some());
    // Staff see it in the list of rides to arrange.
    let listed = not_booked(&w.s).await;
    let mine = listed
        .iter()
        .find(|r| r["appointment_id"] == a.id.to_string())
        .expect("listed");
    assert_eq!(mine["status"], "no_provider");
    assert_eq!(mine["person_id"], w.person.to_string());

    // Cancelling the appointment closes it, with nothing sent anywhere.
    w.s.scheduling
        .cancel(
            a.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(
        w.s.transport
            .booking_for(a.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "cancelled"
    );
    assert!(
        !not_booked(&w.s)
            .await
            .iter()
            .any(|r| r["appointment_id"] == a.id.to_string())
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_ride_the_stand_in_made_up_is_unconfirmed_and_an_upcoming_one_gets_a_call() {
    let w = world(true).await;
    let upcoming = w.book().await;
    let other = common::free_slot(&w.s.db, w.schedule, 9).await;
    let past =
        w.s.scheduling
            .book_slot(other.id, w.person, &sys())
            .await
            .unwrap();
    w.s.db
        .run(Stmt::new(
            "UPDATE appointments SET start_datetime = now() - interval '2 days', end_datetime = now() - interval '2 days' + interval '15 minutes' WHERE id = $1",
            [past.id.into()],
        ))
        .await
        .unwrap();
    for a in [upcoming.id, past.id] {
        let id = Uuid::new_v4();
        w.s.db
            .run(Stmt::new(
                "INSERT INTO transport_bookings (id, appointment_id, person_id, status, provider_ref, window_start, window_end) \
                 VALUES ($1,$2,$3,'booked',$4, now(), now() + interval '30 minutes')",
                [id.into(), a.into(), w.person.into(), format!("stub-{id}").into()],
            ))
            .await
            .unwrap();
    }
    // Migration 75 again: every statement in it can run twice.
    let sql = include_str!("../migrations/sql/20261128000075_transport_no_provider.up.sql");
    let body: String = sql
        .lines()
        .filter(|l| !l.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    for stmt in body.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        w.s.db.run(Stmt::text(stmt.to_string())).await.unwrap();
    }
    for a in [upcoming.id, past.id] {
        let b = w.s.transport.booking_for(a).await.unwrap().unwrap();
        assert_eq!(b.status, "unconfirmed");
        assert_eq!(
            (b.window_start, b.window_end),
            (None, None),
            "the made-up window is gone"
        );
        assert!(
            b.provider_ref.unwrap().starts_with("stub-"),
            "kept, to match what the patient was told"
        );
    }
    let calls = |a: Uuid| {
        let db = w.s.db.clone();
        async move {
            db.one(Stmt::new(
                "SELECT count(*)::bigint AS n FROM staff_call_tasks WHERE appointment_id = $1 AND reason = 'transport_unconfirmed' AND handled_at IS NULL",
                [a.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .col::<i64>("n")
            .unwrap()
        }
    };
    assert_eq!(calls(upcoming.id).await, 1);
    assert_eq!(calls(past.id).await, 0, "a past appointment needs no ride");
    let listed = not_booked(&w.s).await;
    assert!(listed.iter().any(|r| r["appointment_id"] == upcoming.id.to_string() && r["status"] == "unconfirmed"));
    assert!(
        !listed
            .iter()
            .any(|r| r["appointment_id"] == past.id.to_string())
    );
    // A confirmation now books nothing new over it: the appointment still has its one open booking.
    assert_eq!(w.booked(), 0);
}
