//! CAP-14: reply to turn an appointment into a video or phone consultation.
//! Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test uses its own service name and patient, so approvals and appointments are its own.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::ApiResponse;
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{
    Appointment, AppointmentStatus, BarrierCode, ConversionState, Modality,
};
use patient_administration_system::virtual_visits::{ApprovedService, Review};
use patient_administration_system::writeback::{ReplyAction, ReplyRequest, ReplyStatus};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    service: String,
    person: Uuid,
    appt: Appointment,
}

/// A patient booked a few days ahead on a service of their own.
async fn world() -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("clinic-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    let slot = common::free_slot(&s.db, schedule, 4).await;
    let person = Uuid::new_v4();
    let appt = s
        .scheduling
        .book_slot(slot.id, person, &sys())
        .await
        .unwrap();
    W {
        s,
        service,
        person,
        appt,
    }
}

fn convert(w: &W, id: &str, modality: Modality) -> ReplyRequest {
    ReplyRequest {
        message_id: id.to_string(),
        action: ReplyAction::Convert,
        appointment_id: w.appt.id,
        person_id: w.person,
        channel: Some("sms".into()),
        barrier_code: Some(BarrierCode::CannotTravel),
        new_slot_id: None,
        modality: Some(modality),
        received_at: None,
    }
}

async fn reload(w: &W) -> Appointment {
    AppointmentRepository::find_by_id(&w.s.db, w.appt.id)
        .await
        .unwrap()
        .unwrap()
}

async fn approve(w: &W) {
    w.s.virtual_visits
        .approve_service(&w.service, "Dr A. Clinician", &sys())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_service_a_clinician_has_not_approved_cannot_go_virtual() {
    let w = world().await;
    let r =
        w.s.writeback
            .apply(
                convert(&w, &format!("m-{}", Uuid::new_v4()), Modality::Video),
                &sys(),
            )
            .await
            .unwrap();
    assert_eq!(r.status, ReplyStatus::Rejected);
    assert_eq!(r.reject_reason.as_deref(), Some("not_virtual_ok"));
    let a = reload(&w).await;
    assert_eq!(a.modality, Modality::InPerson, "the slot is untouched");
    assert_eq!(a.conversion_state, None);
    assert_eq!(a.status, AppointmentStatus::Booked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_approval_must_name_a_person() {
    let w = world().await;
    for blank in ["", "   "] {
        assert!(
            w.s.virtual_visits
                .approve_service(&w.service, blank, &sys())
                .await
                .is_err()
        );
    }
    assert!(
        w.s.virtual_visits
            .list_services()
            .await
            .unwrap()
            .iter()
            .all(|x| x.service_type != w.service)
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn convert_changes_modality_once_records_the_barrier_and_is_idempotent_by_message_id() {
    let w = world().await;
    approve(&w).await;
    let id = format!("m-{}", Uuid::new_v4());
    let r =
        w.s.writeback
            .apply(convert(&w, &id, Modality::Video), &sys())
            .await
            .unwrap();
    assert_eq!(r.status, ReplyStatus::Applied, "{:?}", r.reject_reason);
    let a = reload(&w).await;
    assert_eq!(a.modality, Modality::Video);
    assert_eq!(a.conversion_state, Some(ConversionState::PendingReview));
    assert_eq!(a.conversion_barrier_code, Some(BarrierCode::CannotTravel));
    assert_eq!(a.slot_id, w.appt.slot_id, "same slot");
    assert_eq!(a.status, AppointmentStatus::Booked);
    assert_eq!(
        common::audit_count(&w.s.db, w.appt.id, "reply_convert").await,
        1,
        "the audit log records it"
    );

    // The same message again is the same outcome, not a second conversion.
    let again =
        w.s.writeback
            .apply(convert(&w, &id, Modality::Video), &sys())
            .await
            .unwrap();
    assert!(again.duplicate);
    assert_eq!(
        common::audit_count(&w.s.db, w.appt.id, "reply_convert").await,
        1
    );
    // A different message asking for the same thing changes nothing more.
    let same =
        w.s.writeback
            .apply(
                convert(&w, &format!("m-{}", Uuid::new_v4()), Modality::Video),
                &sys(),
            )
            .await
            .unwrap();
    assert_eq!(same.status, ReplyStatus::Applied);
    assert_eq!(
        common::audit_count(&w.s.db, w.appt.id, "reply_convert").await,
        1
    );
    // Asking for something else once converted is refused.
    let other =
        w.s.writeback
            .apply(
                convert(&w, &format!("m-{}", Uuid::new_v4()), Modality::Phone),
                &sys(),
            )
            .await
            .unwrap();
    assert_eq!(other.reject_reason.as_deref(), Some("already_converted"));
    assert_eq!(reload(&w).await.modality, Modality::Video);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn only_a_booked_appointment_to_come_can_be_converted() {
    let w = world().await;
    approve(&w).await;
    w.s.scheduling
        .cancel(
            w.appt.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &sys(),
        )
        .await
        .unwrap();
    let r =
        w.s.writeback
            .apply(
                convert(&w, &format!("m-{}", Uuid::new_v4()), Modality::Phone),
                &sys(),
            )
            .await
            .unwrap();
    assert_eq!(r.status, ReplyStatus::Rejected);
    assert!(r.reject_reason.unwrap().starts_with("not_convertible"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn another_patient_cannot_convert_this_appointment() {
    let w = world().await;
    approve(&w).await;
    let mut req = convert(&w, &format!("m-{}", Uuid::new_v4()), Modality::Video);
    req.person_id = Uuid::new_v4();
    let r = w.s.writeback.apply(req, &sys()).await.unwrap();
    assert_eq!(r.reject_reason.as_deref(), Some("person_mismatch"));
    assert_eq!(reload(&w).await.modality, Modality::InPerson);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_convert_reply_must_say_video_or_phone() {
    let w = world().await;
    let mut req = convert(&w, "x", Modality::Video);
    req.modality = None;
    assert!(w.s.writeback.apply(req.clone(), &sys()).await.is_err());
    req.modality = Some(Modality::InPerson);
    assert!(w.s.writeback.apply(req, &sys()).await.is_err());
    // And modality means nothing on a confirm.
    let mut c = convert(&w, "y", Modality::Video);
    c.action = ReplyAction::Confirm;
    c.barrier_code = None;
    assert!(w.s.writeback.apply(c, &sys()).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_clinician_can_confirm_or_veto_and_a_veto_restores_in_person() {
    let w = world().await;
    approve(&w).await;
    w.s.writeback
        .apply(
            convert(&w, &format!("m-{}", Uuid::new_v4()), Modality::Phone),
            &sys(),
        )
        .await
        .unwrap();
    assert!(
        w.s.virtual_visits
            .pending(500)
            .await
            .unwrap()
            .iter()
            .any(|a| a.id == w.appt.id)
    );

    let vetoed =
        w.s.virtual_visits
            .review(w.appt.id, Review::Veto, &sys())
            .await
            .unwrap();
    assert_eq!(vetoed.modality, Modality::InPerson);
    assert_eq!(vetoed.conversion_state, Some(ConversionState::Vetoed));
    assert_eq!(vetoed.status, AppointmentStatus::Booked, "the slot is kept");
    // Reviewed once.
    assert!(
        w.s.virtual_visits
            .review(w.appt.id, Review::Confirm, &sys())
            .await
            .is_err()
    );
    assert!(
        w.s.virtual_visits
            .pending(500)
            .await
            .unwrap()
            .iter()
            .all(|a| a.id != w.appt.id)
    );

    // After a veto the patient may ask again, and a clinician may then confirm.
    let again =
        w.s.writeback
            .apply(
                convert(&w, &format!("m-{}", Uuid::new_v4()), Modality::Video),
                &sys(),
            )
            .await
            .unwrap();
    assert_eq!(again.status, ReplyStatus::Applied);
    let ok =
        w.s.virtual_visits
            .review(w.appt.id, Review::Confirm, &sys())
            .await
            .unwrap();
    assert_eq!(ok.modality, Modality::Video);
    assert_eq!(ok.conversion_state, Some(ConversionState::Confirmed));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_approve_review_and_list() {
    let w = world().await;
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
    let uri = format!("/api/virtual-ok-services/{}", w.service);
    let (st, _) = call(
        "PUT",
        uri.clone(),
        Some(serde_json::json!({ "approved_by": " " })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "an approval needs a name");
    let (st, v) = call(
        "PUT",
        uri.clone(),
        Some(serde_json::json!({ "approved_by": "Dr A. Clinician" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["approved_by"], "Dr A. Clinician");
    // QUAL-34: the answer is the type the route declares.
    let approved: ApiResponse<ApprovedService> = serde_json::from_value(v).unwrap();
    assert_eq!(approved.data.unwrap().service_type, w.service);
    let (st, v) = call("GET", "/api/virtual-ok-services".into(), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let listed: ApiResponse<Vec<ApprovedService>> = serde_json::from_value(v).unwrap();
    assert!(
        listed
            .data
            .unwrap()
            .iter()
            .any(|s| s.service_type == w.service)
    );

    // The reply goes in through the write-back endpoint like any other.
    let (st, v) = call(
        "POST",
        "/api/write-back/replies".into(),
        Some(serde_json::json!({
            "message_id": format!("m-{}", Uuid::new_v4()), "action": "convert",
            "appointment_id": w.appt.id, "person_id": w.person,
            "modality": "video", "barrier_code": "caring",
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["status"], "applied");
    assert_eq!(v["data"]["appointment"]["modality"], "video");

    let (st, v) = call("GET", "/api/conversions/pending?limit=500".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    let pending: ApiResponse<Vec<Appointment>> = serde_json::from_value(v).unwrap();
    assert!(pending.data.unwrap().iter().any(|a| a.id == w.appt.id));
    let (st, _) = call(
        "POST",
        format!("/api/appointments/{}/conversion/maybe", w.appt.id),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        "POST",
        format!("/api/appointments/{}/conversion/veto", w.appt.id),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let vetoed: ApiResponse<Appointment> = serde_json::from_value(v).unwrap();
    assert_eq!(vetoed.data.unwrap().modality, Modality::InPerson);

    // Withdrawing the approval stops further conversions.
    let (st, _) = call("DELETE", uri, None).await;
    assert_eq!(st, StatusCode::OK);
    let r =
        w.s.writeback
            .apply(
                convert(&w, &format!("m-{}", Uuid::new_v4()), Modality::Phone),
                &sys(),
            )
            .await
            .unwrap();
    assert_eq!(r.reject_reason.as_deref(), Some("not_virtual_ok"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_video_visit_that_fails_falls_back_to_a_phone_call_once() {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    let w = world().await;
    // An in-person appointment cannot "fall back".
    assert!(
        w.s.virtual_visits
            .fall_back_to_phone(w.appt.id, &sys())
            .await
            .is_err()
    );

    approve(&w).await;
    let r =
        w.s.writeback
            .apply(
                convert(&w, &format!("v-{}", Uuid::new_v4()), Modality::Video),
                &sys(),
            )
            .await
            .unwrap();
    assert_eq!(r.status, ReplyStatus::Applied);
    assert_eq!(reload(&w).await.modality, Modality::Video);

    let a =
        w.s.virtual_visits
            .fall_back_to_phone(w.appt.id, &sys())
            .await
            .unwrap();
    assert_eq!(a.modality, Modality::Phone);
    assert_eq!(
        a.status,
        AppointmentStatus::Booked,
        "the appointment itself stands"
    );
    let tasks = |s: &AppState, id: Uuid| {
        let db = s.db.clone();
        async move {
            db.query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS n FROM staff_call_tasks WHERE appointment_id = $1 AND reason = 'video_failed' AND handled_at IS NULL",
                [id.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "n")
            .unwrap()
        }
    };
    assert_eq!(tasks(&w.s, w.appt.id).await, 1, "a person is asked to ring");
    // A second report (the provider retries) changes nothing and adds no second task.
    w.s.virtual_visits
        .fall_back_to_phone(w.appt.id, &sys())
        .await
        .unwrap();
    assert_eq!(tasks(&w.s, w.appt.id).await, 1);
    assert!(
        w.s.ladder
            .open_calls(500)
            .await
            .unwrap()
            .iter()
            .any(|c| c["appointment_id"] == w.appt.id.to_string() && c["reason"] == "video_failed")
    );

    // Over HTTP.
    let w2 = world().await;
    approve(&w2).await;
    w2.s.writeback
        .apply(
            convert(&w2, &format!("v-{}", Uuid::new_v4()), Modality::Video),
            &sys(),
        )
        .await
        .unwrap();
    let app = patient_administration_system::api::rest::router(w2.s.clone());
    let r = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/appointments/{}/video-failed", w2.appt.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let fell_back: ApiResponse<Appointment> =
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(fell_back.data.unwrap().modality, Modality::Phone);
    assert_eq!(reload(&w2).await.modality, Modality::Phone);
}
