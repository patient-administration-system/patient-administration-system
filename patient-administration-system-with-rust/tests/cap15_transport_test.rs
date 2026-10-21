//! CAP-15: non-emergency transport booked from a confirmed appointment.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::Duration;
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
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
            window_start: req.appointment_start - Duration::minutes(80),
            window_end: req.appointment_start - Duration::minutes(50),
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
        .with_person_client(stub)
        .with_transport_client(rec.clone());
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
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        w.s.db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM transport_bookings WHERE last_error LIKE '%Mill Lane%'"
                    .to_string(),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "n")
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
    let calls = w.s.ladder.open_calls(500).await.unwrap();
    assert!(
        calls
            .iter()
            .any(|c| c["appointment_id"] == a.id.to_string() && c["reason"] == "transport_failed")
    );

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
    let calls = w.s.ladder.open_calls(500).await.unwrap();
    assert!(
        calls
            .iter()
            .any(|c| c["appointment_id"] == a.id.to_string() && c["reason"] == "transport_failed")
    );
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
