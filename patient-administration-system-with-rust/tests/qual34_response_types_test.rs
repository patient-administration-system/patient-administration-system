//! QUAL-34: the `data` type each route declares in the OpenAPI document is what the route sends. For each
//! typed route the response body is read back into the declared Rust type through `ApiResponse<T>`, so a
//! field renamed or dropped on either side fails here. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::handlers_contact::{
    NhsNumberChecked, OptOut, OptOutState,
};
use patient_administration_system::api::rest::router;
use patient_administration_system::api::{ApiResponse, Nothing};
use patient_administration_system::contact::Profile;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{Appointment, CancelledBy};
use patient_administration_system::models::rtt::{RTTClockEvent, RTTPathway};
use patient_administration_system::models::waitlist::WaitlistEntry;
use patient_administration_system::scheduling::CancelDetails;
use patient_administration_system::standby::{Releases, StandbyEntry};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn send(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> Vec<u8> {
    let mut req = Request::builder().method(method).uri(uri);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let res = app
        .clone()
        .oneshot(
            req.body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 22).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{method} {uri}: {}",
        String::from_utf8_lossy(&bytes)
    );
    bytes.to_vec()
}

async fn call<T: DeserializeOwned>(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> T {
    let bytes = send(app, method, uri, body).await;
    let envelope: ApiResponse<T> = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("{method} {uri} is not the declared type: {e}"));
    assert!(envelope.success, "{method} {uri}");
    envelope
        .data
        .unwrap_or_else(|| panic!("{method} {uri}: no data"))
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_waitlist_and_rtt_routes_send_the_types_they_declare() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let service = format!("svc-{}", Uuid::new_v4());
    let person = Uuid::new_v4();

    let entry: WaitlistEntry = call(
        &app,
        "POST",
        "/api/waitlist",
        Some(json!({"person_id": person, "target_service": service, "priority": "urgent"})),
    )
    .await;
    let listed: Vec<WaitlistEntry> = call(
        &app,
        "GET",
        &format!("/api/waitlist?service={service}"),
        None,
    )
    .await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, entry.id);

    let sched = common::new_schedule_for(&state.db, &service).await;
    let slot = common::free_slot(&state.db, sched.id, 12).await;
    let booked: Appointment = call(
        &app,
        "POST",
        &format!("/api/waitlist/{}/book", entry.id),
        Some(json!({"slot_id": slot.id})),
    )
    .await;
    assert_eq!(booked.person_id, person);

    let second: WaitlistEntry = call(
        &app,
        "POST",
        "/api/waitlist",
        Some(
            json!({"person_id": Uuid::new_v4(), "target_service": service, "priority": "routine"}),
        ),
    )
    .await;
    let removed: WaitlistEntry = call(
        &app,
        "DELETE",
        &format!("/api/waitlist/{}", second.id),
        None,
    )
    .await;
    assert_eq!(removed.id, second.id);

    let pathway: RTTPathway = call(
        &app,
        "POST",
        "/api/rtt/start",
        Some(json!({"person_id": person, "target_service": service})),
    )
    .await;
    for step in ["pause", "resume"] {
        let _: RTTClockEvent = call(
            &app,
            "POST",
            &format!("/api/rtt/{}/{step}", pathway.id),
            Some(json!({"reason": "patient away"})),
        )
        .await;
    }
    let _: RTTClockEvent = call(
        &app,
        "POST",
        &format!("/api/rtt/{}/stop", pathway.id),
        Some(json!({"reason": "treated"})),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_appointment_routes_send_the_types_they_declare() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let person = Uuid::new_v4();
    let sched = common::new_schedule(&state.db).await;
    let slot = common::free_slot(&state.db, sched.id, 15).await;

    let appt: Appointment = call(
        &app,
        "POST",
        &format!("/api/slots/{}/book", slot.id),
        Some(json!({"person_id": person})),
    )
    .await;
    let day = slot.start_datetime.format("%Y-%m-%dT00:00:00Z");
    let next = (slot.start_datetime + chrono::Duration::days(1)).format("%Y-%m-%dT00:00:00Z");
    let in_range: Vec<Appointment> = call(
        &app,
        "GET",
        &format!("/api/appointments?from={day}&to={next}"),
        None,
    )
    .await;
    assert!(in_range.iter().any(|a| a.id == appt.id));
    let mine: Vec<Appointment> = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/appointments"),
        None,
    )
    .await;
    assert_eq!(mine.len(), 1);
    let _: Appointment = call(
        &app,
        "POST",
        &format!("/api/appointments/{}/check-in", appt.id),
        None,
    )
    .await;
    let _: Appointment = call(
        &app,
        "POST",
        &format!("/api/appointments/{}/complete", appt.id),
        None,
    )
    .await;
}

/// A response whose type only serialises cannot be read back into it, so its fields are compared with the
/// schema the OpenAPI document declares: every field sent is declared, and every declared required field is sent.
fn assert_fields_match_schema(schema_name: &str, sent: &Value) {
    use utoipa::OpenApi;
    let doc = serde_json::to_value(patient_administration_system::api::openapi::ApiDoc::openapi())
        .unwrap();
    let schema = &doc["components"]["schemas"][schema_name];
    let declared = schema["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("no schema {schema_name}"));
    let object = sent
        .as_object()
        .unwrap_or_else(|| panic!("{schema_name}: not an object: {sent}"));
    for key in object.keys() {
        assert!(
            declared.contains_key(key),
            "{schema_name}: `{key}` is sent but not declared"
        );
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let key = required.as_str().unwrap();
        assert!(
            object.contains_key(key),
            "{schema_name}: required `{key}` is not sent"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_tolerance_and_letter_template_routes_send_the_fields_they_declare() {
    use patient_administration_system::db::repositories::letter::LetterRepository;
    use patient_administration_system::models::communication::LetterTemplate;

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());

    let status: Value = call(&app, "GET", "/api/tolerances/status?days=30", None).await;
    assert_fields_match_schema("ToleranceStatus", &status);

    let alerts: Vec<Value> = call(&app, "GET", "/api/tolerance-alerts", None).await;
    for a in alerts.iter().take(3) {
        assert_fields_match_schema("Alert", a);
    }

    let t = LetterTemplate::new(
        format!("q34-{}", Uuid::new_v4()),
        "Hello".into(),
        "Your appointment.".into(),
    );
    let t = LetterRepository::create_template(&state.db, &t)
        .await
        .unwrap();
    let one: Value = call(
        &app,
        "GET",
        &format!("/api/letter-templates/{}", t.id),
        None,
    )
    .await;
    assert_fields_match_schema("LetterTemplate", &one);
    let all: Vec<Value> = call(&app, "GET", "/api/letter-templates", None).await;
    assert!(all.iter().any(|x| x["id"] == one["id"]));
    for x in all.iter().take(3) {
        assert_fields_match_schema("LetterTemplate", x);
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_history_due_check_adjustment_and_noshow_routes_send_the_types_they_declare() {
    use patient_administration_system::api::rest::handlers_adjustments::AppointmentAdjustments;
    use patient_administration_system::api::rest::handlers_waits::DueCheck;
    use patient_administration_system::history::HistoryRow;

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let person = Uuid::new_v4();
    let sched = common::new_schedule(&state.db).await;
    let slot = common::free_slot(&state.db, sched.id, 18).await;
    let appt: Appointment = call(
        &app,
        "POST",
        &format!("/api/slots/{}/book", slot.id),
        Some(json!({"person_id": person})),
    )
    .await;

    let history: Vec<HistoryRow> = call(
        &app,
        "GET",
        &format!("/api/appointments/{}/history", appt.id),
        None,
    )
    .await;
    assert!(history.iter().any(|h| h.kind.as_deref() == Some("audit")));
    let _log: Vec<HistoryRow> = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/contact-log"),
        None,
    )
    .await;

    let service = format!("svc-{}", Uuid::new_v4());
    let pathway: RTTPathway = call(
        &app,
        "POST",
        "/api/rtt/start",
        Some(json!({"person_id": person, "target_service": service})),
    )
    .await;
    let due: Vec<DueCheck> = call(
        &app,
        "GET",
        "/api/pathways/due-check?weeks=0&repeat_days=1",
        None,
    )
    .await;
    assert!(
        due.iter()
            .any(|d| d.pathway_id == pathway.id && d.person_id == person)
    );

    let asked: patient_administration_system::api::rest::handlers_waits::Asked = call(
        &app,
        "POST",
        &format!("/api/pathways/{}/still-needed-checks", pathway.id),
        Some(json!({"asked_by": "Nurse B"})),
    )
    .await;
    assert_ne!(asked.id, Uuid::nil());
    let proofed: patient_administration_system::api::rest::handlers_identity::Proofed = call(
        &app,
        "GET",
        &format!("/api/identity-proofings/qual34-{person}/check?person_id={person}"),
        None,
    )
    .await;
    assert!(!proofed.proofed, "nobody has proofed this account");

    let _adjustments: Vec<AppointmentAdjustments> = call(
        &app,
        "GET",
        &format!(
            "/api/reasonable-adjustments/for-appointments?ids={}",
            appt.id
        ),
        None,
    )
    .await;
    let shown: Value = call(
        &app,
        "GET",
        &format!("/api/noshow/for-appointments?ids={}", appt.id),
        None,
    )
    .await;
    assert_fields_match_schema("ShownAssessments", &shown);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_referral_routes_send_the_types_they_declare() {
    use patient_administration_system::referrals::{Booked, Created, Referral, ReferralStatus};

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let service = format!("svc-{}", Uuid::new_v4());
    let new = |n: u32| {
        json!({"person_id": Uuid::new_v4(), "source": "gp", "external_ref": format!("q34-{}-{n}", Uuid::new_v4()),
               "target_service": service, "reason": "knee"})
    };

    let created: Created = call(&app, "POST", "/api/referrals", Some(new(1))).await;
    assert!(!created.duplicate);
    let id = created.referral.id;
    let one: Referral = call(&app, "GET", &format!("/api/referrals/{id}"), None).await;
    assert_eq!(one.status, ReferralStatus::Received);
    let listed: Vec<Referral> = call(
        &app,
        "GET",
        &format!("/api/referrals?service={service}"),
        None,
    )
    .await;
    assert!(listed.iter().any(|r| r.id == id));

    let waitlisted: Referral = call(
        &app,
        "POST",
        &format!("/api/referrals/{id}/waitlist"),
        Some(json!({"priority": "urgent"})),
    )
    .await;
    assert_eq!(waitlisted.status, ReferralStatus::Waitlisted);
    let closed: Referral = call(
        &app,
        "POST",
        &format!("/api/referrals/{id}/close"),
        Some(json!({"reason": "no longer needed"})),
    )
    .await;
    assert_eq!(closed.status, ReferralStatus::Closed);

    let second: Created = call(&app, "POST", "/api/referrals", Some(new(2))).await;
    let triaged: Referral = call(
        &app,
        "POST",
        &format!("/api/referrals/{}/triage", second.referral.id),
        Some(json!({"outcome": "returned_with_advice", "triaged_by": "Dr B"})),
    )
    .await;
    assert_eq!(triaged.status, ReferralStatus::ReturnedWithAdvice);

    let third: Created = call(&app, "POST", "/api/referrals", Some(new(3))).await;
    let sched = common::new_schedule_for(&state.db, &service).await;
    let _slot = common::free_slot(&state.db, sched.id, 20).await;
    let booked: Booked = call(
        &app,
        "POST",
        &format!("/api/referrals/{}/book-first-available", third.referral.id),
        None,
    )
    .await;
    assert_eq!(booked.referral.status, ReferralStatus::Booked);
    assert_eq!(booked.referral.appointment_id, Some(booked.appointment.id));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_patient_record_routes_send_the_types_they_declare() {
    use patient_administration_system::adjustments::Adjustment;
    use patient_administration_system::db::repositories::{
        coverage::CoverageRepository, encounter::EncounterRepository,
    };
    use patient_administration_system::models::consent::{Consent, ConsentType};
    use patient_administration_system::models::coverage::Coverage;
    use patient_administration_system::models::encounter::{Encounter, EncounterClass};
    use patient_administration_system::proxy::Authority;
    use patient_administration_system::waits::PatientWaits;

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let person = Uuid::new_v4();
    let today = chrono::Utc::now().date_naive();

    let consent = Consent::new(person, ConsentType::DataProcessing, today);
    let made: Consent = call(
        &app,
        "POST",
        &format!("/api/patients/{person}/consents"),
        Some(serde_json::to_value(&consent).unwrap()),
    )
    .await;
    assert_eq!(made.person_id, person);
    let consents: Vec<Consent> = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/consents"),
        None,
    )
    .await;
    assert_eq!(consents.len(), 1);

    CoverageRepository::create(&state.db, &Coverage::new(person, "Payer", "P-1"))
        .await
        .unwrap();
    let coverages: Vec<Coverage> = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/coverages"),
        None,
    )
    .await;
    assert_eq!(coverages.len(), 1);

    EncounterRepository::create(
        &state.db,
        &Encounter::new(person, EncounterClass::Outpatient),
    )
    .await
    .unwrap();
    let encounters: Vec<Encounter> = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/encounters"),
        None,
    )
    .await;
    assert_eq!(encounters.len(), 1);

    let granted: Authority = call(
        &app,
        "POST",
        &format!("/api/patients/{person}/proxies"),
        Some(
            json!({"proxy_person_id": Uuid::new_v4(), "relationship": "daughter",
                    "basis": "patient_consent", "scopes": ["receive", "confirm"],
                    "expires_at": chrono::Utc::now() + chrono::Duration::days(30),
                    "recorded_by": "Nurse A"}),
        ),
    )
    .await;
    assert_eq!(granted.person_id, person);
    let proxies: Vec<Authority> = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/proxies"),
        None,
    )
    .await;
    assert_eq!(proxies.len(), 1);

    let adjustment: Adjustment = call(
        &app,
        "POST",
        &format!("/api/patients/{person}/reasonable-adjustments"),
        Some(json!({"code": "large_print", "recorded_by": "Nurse A"})),
    )
    .await;
    assert_eq!(adjustment.code, "large_print");
    let adjustments: Vec<Adjustment> = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/reasonable-adjustments"),
        None,
    )
    .await;
    assert_eq!(adjustments.len(), 1);

    let waits: PatientWaits =
        call(&app, "GET", &format!("/api/patients/{person}/waits"), None).await;
    assert_eq!(waits.person_id, person);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_admission_routes_send_the_types_they_declare() {
    use patient_administration_system::adt::AdmissionResult;
    use patient_administration_system::api::rest::handlers_adt::{Reinstated, Reverted};
    use patient_administration_system::models::Address;
    use patient_administration_system::models::admission::{Admission, Discharge, Transfer};
    use patient_administration_system::models::encounter::Encounter;
    use patient_administration_system::models::facility::{Bed, Facility, Room, Ward};

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let unique = Uuid::new_v4().simple().to_string();
    let facility = Facility::new("F".into(), format!("F{unique}"), Address::default());
    let _: Value = call(&app, "POST", "/api/facilities", Some(json!(facility))).await;
    let ward = Ward::new(facility.id, "W".into(), format!("W{unique}"), 4);
    let _: Value = call(&app, "POST", "/api/wards", Some(json!(ward))).await;
    let room = Room::new(ward.id, "R".into(), format!("R{unique}"));
    let _: Value = call(&app, "POST", "/api/rooms", Some(json!(room))).await;
    let mut beds = Vec::new();
    for n in 0..4 {
        let bed = Bed::new(room.id, format!("B{n}"), format!("B{n}{unique}"));
        let _: Value = call(&app, "POST", "/api/beds", Some(json!(bed))).await;
        beds.push(bed.id);
    }
    let person = Uuid::new_v4();
    let at = |bed: Uuid, who: Uuid| json!({"person_id": who, "bed_id": bed});

    let _: Encounter = call(
        &app,
        "POST",
        "/api/admissions/pre-admit",
        Some(at(beds[0], person)),
    )
    .await;
    let _: Encounter = call(
        &app,
        "POST",
        "/api/admissions/cancel-pre-admit",
        Some(at(beds[0], person)),
    )
    .await;

    let admitted: AdmissionResult =
        call(&app, "POST", "/api/admissions", Some(at(beds[0], person))).await;
    let id = admitted.admission.id;
    let _: Encounter = call(
        &app,
        "POST",
        &format!("/api/admissions/{id}/leave-start"),
        None,
    )
    .await;
    let _: Encounter = call(
        &app,
        "POST",
        &format!("/api/admissions/{id}/leave-end"),
        None,
    )
    .await;

    let moved: Transfer = call(
        &app,
        "POST",
        &format!("/api/admissions/{id}/transfer"),
        Some(json!({"new_bed_id": beds[1]})),
    )
    .await;
    assert_eq!(moved.to_bed_id, beds[1]);
    let undone: Reverted = call(
        &app,
        "POST",
        &format!("/api/admissions/{id}/cancel-transfer"),
        None,
    )
    .await;
    assert!(undone.reverted);

    let out: Discharge = call(
        &app,
        "POST",
        &format!("/api/admissions/{id}/discharge"),
        None,
    )
    .await;
    assert_eq!(out.admission_id, id);
    let back: Reinstated = call(
        &app,
        "POST",
        &format!("/api/admissions/{id}/cancel-discharge"),
        None,
    )
    .await;
    assert!(back.reinstated);

    let _: Encounter = call(
        &app,
        "POST",
        &format!("/api/admissions/{id}/change-to-outpatient"),
        None,
    )
    .await;
    let _: AdmissionResult = call(
        &app,
        "POST",
        "/api/admissions/change-to-inpatient",
        Some(at(beds[3], person)),
    )
    .await;

    let other = Uuid::new_v4();
    let second: AdmissionResult =
        call(&app, "POST", "/api/admissions", Some(at(beds[2], other))).await;
    let cancelled: Admission = call(
        &app,
        "POST",
        &format!("/api/admissions/{}/cancel-admit", second.admission.id),
        None,
    )
    .await;
    assert_eq!(cancelled.id, second.admission.id);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_noshow_status_and_rollout_routes_send_what_they_declare() {
    use patient_administration_system::noshow::RolloutRow;

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());

    let status: Value = call(&app, "GET", "/api/noshow/status", None).await;
    assert_fields_match_schema("noshow.Status", &status);
    let rollout: Vec<RolloutRow> = call(&app, "GET", "/api/noshow/rollout", None).await;
    let _ = rollout;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_facility_ward_room_and_bed_routes_send_the_types_they_declare() {
    use patient_administration_system::models::Address;
    use patient_administration_system::models::facility::{Bed, Facility, Room, Ward};
    use patient_administration_system::resources::WardOccupancy;

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let unique = Uuid::new_v4().simple().to_string();

    let facility = Facility::new("F".into(), format!("F{unique}"), Address::default());
    let made: Facility = call(&app, "POST", "/api/facilities", Some(json!(facility))).await;
    let _: Facility = call(&app, "GET", &format!("/api/facilities/{}", made.id), None).await;
    let all: Vec<Facility> = call(&app, "GET", "/api/facilities", None).await;
    assert!(all.iter().any(|f| f.id == made.id));

    let ward = Ward::new(made.id, "W".into(), format!("W{unique}"), 4);
    let ward: Ward = call(&app, "POST", "/api/wards", Some(json!(ward))).await;
    let _: Ward = call(&app, "GET", &format!("/api/wards/{}", ward.id), None).await;
    let wards: Vec<Ward> = call(
        &app,
        "GET",
        &format!("/api/facilities/{}/wards", made.id),
        None,
    )
    .await;
    assert_eq!(wards.len(), 1);

    let room = Room::new(ward.id, "R".into(), format!("R{unique}"));
    let room: Room = call(&app, "POST", "/api/rooms", Some(json!(room))).await;
    let _: Room = call(&app, "GET", &format!("/api/rooms/{}", room.id), None).await;
    let rooms: Vec<Room> = call(&app, "GET", &format!("/api/wards/{}/rooms", ward.id), None).await;
    assert_eq!(rooms.len(), 1);

    let bed = Bed::new(room.id, "B".into(), format!("B{unique}"));
    let bed: Bed = call(&app, "POST", "/api/beds", Some(json!(bed))).await;
    let _: Bed = call(&app, "GET", &format!("/api/beds/{}", bed.id), None).await;
    let beds: Vec<Bed> = call(&app, "GET", &format!("/api/wards/{}/beds", ward.id), None).await;
    assert_eq!(beds.len(), 1);
    let _: Bed = call(
        &app,
        "PUT",
        &format!("/api/beds/{}/status", bed.id),
        Some(json!({"status": "out_of_service"})),
    )
    .await;
    let occupancy: WardOccupancy = call(
        &app,
        "GET",
        &format!("/api/wards/{}/occupancy", ward.id),
        None,
    )
    .await;
    assert_eq!(occupancy.total_beds, 1);
    assert_eq!(occupancy.out_of_service, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_letter_routes_send_the_types_they_declare() {
    use patient_administration_system::api::rest::handlers_letters::{Preview, TemplateGroup};
    use patient_administration_system::communication::sms_length::{Encoding, SmsLength};
    use patient_administration_system::models::communication::{
        DeliveryChannel, GeneratedLetter, LetterTemplate,
    };
    use patient_administration_system::upstream::{Person, StubPersonClient};
    use std::sync::Arc;

    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        json!({ "given": "Ada", "family": "Lovelace" }),
    );
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let state = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub);
    let app = router(state);

    let name = format!("q34-{}", Uuid::new_v4());
    let template: LetterTemplate = call(
        &app,
        "POST",
        "/api/letter-templates",
        Some(json!({
            "id": Uuid::nil(), "name": name, "locale": "en-001", "subject": "Hello {{ patient.name.given }}",
            "body_tera": "See you soon.", "channels": ["print"], "required_variables": [],
            "active": false,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        })),
    )
    .await;
    assert_eq!(template.name, name);

    let preview: Preview = call(
        &app,
        "POST",
        "/api/letters/preview",
        Some(json!({"template_id": template.id, "person_id": person})),
    )
    .await;
    assert_eq!(preview.rendered_subject, "Hello Ada");
    assert_eq!(preview.sms.encoding, Encoding::Gsm7);

    let letter: GeneratedLetter = call(
        &app,
        "POST",
        "/api/letters/generate",
        Some(json!({"template_id": template.id, "person_id": person, "channel": "print"})),
    )
    .await;
    let again: GeneratedLetter =
        call(&app, "GET", &format!("/api/letters/{}", letter.id), None).await;
    assert_eq!(again.id, letter.id);
    assert_eq!(again.channel, DeliveryChannel::Print);
    let sent: GeneratedLetter = call(
        &app,
        "POST",
        &format!("/api/letters/{}/sent", letter.id),
        Some(json!({})),
    )
    .await;
    assert_eq!(sent.id, letter.id);

    let sms: SmsLength = call(
        &app,
        "POST",
        "/api/letter-templates/check-sms",
        Some(json!({"body_tera": "Dydd Mawrth ŵ", "locale": "cy"})),
    )
    .await;
    assert_eq!(sms.encoding, Encoding::Ucs2);
    assert_eq!(sms.forced_by, Some('ŵ'));

    let group: TemplateGroup = call(
        &app,
        "POST",
        "/api/letter-template-groups",
        Some(json!({
            "name": format!("q34g-{}", Uuid::new_v4()),
            "channels": ["sms"],
            "variants": [
                {"locale": "en-001", "subject": "Hi", "body_tera": "See you."},
                {"locale": "cy-001", "subject": "Helo", "body_tera": "Welai chi."},
            ],
        })),
    )
    .await;
    assert_eq!(group.variants.len(), 2);
    assert!(group.variants.iter().all(|v| v.sms.is_some()));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_standby_and_slot_release_routes_send_the_types_they_declare() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&state.db, &service).await.id;
    let ctx = UserContext::system();

    let entry: StandbyEntry = call(
        &app,
        "POST",
        "/api/standby",
        Some(json!({"person_id": Uuid::new_v4(), "service_type": service})),
    )
    .await;
    let listed: Vec<StandbyEntry> = call(
        &app,
        "GET",
        &format!("/api/standby?service_type={service}"),
        None,
    )
    .await;
    assert!(listed.iter().any(|e| e.id == entry.id));

    // A cancelled booking releases its slot.
    let slot = common::free_slot(&state.db, schedule, 6).await;
    let appointment = state
        .scheduling
        .book_slot(slot.id, Uuid::new_v4(), &ctx)
        .await
        .unwrap();
    state
        .scheduling
        .cancel_with(
            appointment.id,
            CancelDetails {
                cancelled_by: CancelledBy::Patient,
                barrier_code: None,
                legacy_reason: None,
            },
            &ctx,
        )
        .await
        .unwrap();
    // A window of the last minute keeps the body small however much the database holds.
    let from = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
    let releases: Releases = call(
        &app,
        "GET",
        &format!(
            "/api/slot-releases?status=open&from={}",
            from.replace('+', "%2B")
        ),
        None,
    )
    .await;
    let release = releases
        .releases
        .iter()
        .find(|r| r.slot_id == slot.id)
        .expect("the cancelled slot is released");
    assert!(releases.summary.total >= 1);

    let candidates: Vec<StandbyEntry> = call(
        &app,
        "GET",
        &format!("/api/slot-releases/{}/candidates", release.id),
        None,
    )
    .await;
    assert!(candidates.iter().all(|c| c.active));

    let booked: Appointment = call(
        &app,
        "POST",
        &format!("/api/standby/{}/book", entry.id),
        Some(json!({"slot_id": slot.id})),
    )
    .await;
    assert_eq!(booked.person_id, entry.person_id);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_contact_profile_and_opt_out_routes_send_the_types_they_declare() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state);
    let person = Uuid::new_v4();
    let url = format!("/api/patients/{person}/communication-profile");

    let put: Profile = call(
        &app,
        "PUT",
        &url,
        Some(json!({"mobile": "+44 7700 900123", "accessibility_needs": ["audio"]})),
    )
    .await;
    assert_eq!(put.row.person_id, person);
    let got: Profile = call(&app, "GET", &url, None).await;
    assert_eq!(got.row.mobile_e164, put.row.mobile_e164);
    let verified: Profile = call(
        &app,
        "POST",
        &format!("{url}/verify-mobile"),
        Some(json!({"method": "portal"})),
    )
    .await;
    assert!(!verified.mobile_stale);
    let _: Vec<Profile> = call(
        &app,
        "GET",
        "/api/communication-profiles/stale?older_than_days=365&limit=5",
        None,
    )
    .await;

    let optouts = format!("/api/patients/{person}/channel-optouts");
    let added: OptOutState = call(&app, "POST", &optouts, Some(json!({"channel": "sms"}))).await;
    assert!(added.opted_out);
    let listed: Vec<OptOut> = call(&app, "GET", &optouts, None).await;
    assert!(listed.iter().any(|o| o.channel == "sms"));
    let removed: OptOutState = call(&app, "DELETE", &format!("{optouts}/sms"), None).await;
    assert!(!removed.opted_out);

    let checked: NhsNumberChecked = call(
        &app,
        "POST",
        "/api/validate/nhs-number",
        Some(json!({"value": "943 476 5919"})),
    )
    .await;
    assert!(checked.valid);
    assert_eq!(checked.digits, "9434765919");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_coverage_routes_send_the_types_they_declare() {
    use patient_administration_system::models::coverage::{Coverage, CoverageStatus};

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state);
    let person = Uuid::new_v4();

    let mut coverage = Coverage::new(person, "Payer", "P-1");
    let created: Coverage = call(
        &app,
        "POST",
        "/api/coverages",
        Some(serde_json::to_value(&coverage).unwrap()),
    )
    .await;
    assert_eq!(created.person_id, person);
    let url = format!("/api/coverages/{}", created.id);
    let got: Coverage = call(&app, "GET", &url, None).await;
    assert_eq!(got.id, created.id);
    coverage.payor_name = "Other payer".into();
    let updated: Coverage = call(
        &app,
        "PUT",
        &url,
        Some(serde_json::to_value(&coverage).unwrap()),
    )
    .await;
    assert_eq!(updated.payor_name, "Other payer");
    let cancelled: Coverage = call(&app, "DELETE", &url, None).await;
    assert_eq!(cancelled.status, CoverageStatus::Cancelled);
    let none: Vec<Coverage> = call(
        &app,
        "GET",
        &format!("/api/accounts/{}/coverages", Uuid::new_v4()),
        None,
    )
    .await;
    assert!(none.is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_encounter_routes_send_the_types_they_declare() {
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::models::encounter::{
        Encounter, EncounterClass, EncounterStatus,
    };

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let person = Uuid::new_v4();
    let first = EncounterRepository::create(
        &state.db,
        &Encounter::new(person, EncounterClass::Outpatient),
    )
    .await
    .unwrap();
    let second = EncounterRepository::create(
        &state.db,
        &Encounter::new(person, EncounterClass::Outpatient),
    )
    .await
    .unwrap();

    let got: Encounter = call(&app, "GET", &format!("/api/encounters/{}", first.id), None).await;
    assert_eq!(got.id, first.id);
    let arrived: Encounter = call(
        &app,
        "PUT",
        &format!("/api/encounters/{}/status", first.id),
        Some(json!({"status": "arrived"})),
    )
    .await;
    assert_eq!(arrived.status, EncounterStatus::Arrived);
    let cancelled: Encounter = call(
        &app,
        "POST",
        &format!("/api/encounters/{}/cancel", second.id),
        None,
    )
    .await;
    assert_eq!(cancelled.status, EncounterStatus::Cancelled);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_series_and_consent_revoke_routes_send_the_types_they_declare() {
    use patient_administration_system::models::appointment_series::{
        AppointmentSeries, SeriesStatus,
    };
    use patient_administration_system::models::consent::{Consent, ConsentStatus, ConsentType};
    use patient_administration_system::scheduling::series::{
        CreateSeriesResult, SeriesWithOccurrences,
    };

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state);
    let person = Uuid::new_v4();

    let start = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();
    let created: CreateSeriesResult = call(
        &app,
        "POST",
        "/api/appointment-series",
        Some(json!({
            "person_id": person,
            "service_type": "physio",
            "start_datetime": start,
            "duration_minutes": 30,
            "rule": {"frequency": "weekly", "interval": 1, "end": {"kind": "count", "count": 3}},
        })),
    )
    .await;
    assert_eq!(created.appointments.len(), 3);
    let url = format!("/api/appointment-series/{}", created.series.id);
    let got: SeriesWithOccurrences = call(&app, "GET", &url, None).await;
    assert_eq!(got.appointments.len(), 3);
    let listed: Vec<AppointmentSeries> = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/appointment-series"),
        None,
    )
    .await;
    assert_eq!(listed.len(), 1);
    let cancelled: SeriesWithOccurrences = call(&app, "POST", &format!("{url}/cancel"), None).await;
    assert_eq!(cancelled.series.status, SeriesStatus::Cancelled);

    let today = chrono::Utc::now().date_naive();
    let consent: Consent = call(
        &app,
        "POST",
        &format!("/api/patients/{person}/consents"),
        Some(
            serde_json::to_value(Consent::new(person, ConsentType::DataProcessing, today)).unwrap(),
        ),
    )
    .await;
    let revoked: Consent = call(
        &app,
        "POST",
        &format!("/api/consents/{}/revoke", consent.id),
        None,
    )
    .await;
    assert_eq!(revoked.status, ConsentStatus::Revoked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_subscription_routes_send_the_types_they_declare() {
    use patient_administration_system::subscriptions::{Subscription, SubscriptionState};

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state);
    let created: Subscription = call(
        &app,
        "POST",
        "/api/admin/subscriptions",
        Some(json!({
            "topic": "appointment",
            "endpoint": "https://example.org/hook",
            "created_by": "qual34",
        })),
    )
    .await;
    assert_eq!(created.status, SubscriptionState::Active);
    let listed: Vec<Subscription> = call(&app, "GET", "/api/admin/subscriptions", None).await;
    assert!(listed.iter().any(|s| s.id == created.id));
    let ended: Subscription = call(
        &app,
        "DELETE",
        &format!("/api/admin/subscriptions/{}", created.id),
        None,
    )
    .await;
    assert_eq!(ended.status, SubscriptionState::Off);
    assert!(ended.ended_at.is_some());
}

/// A success that has nothing to return: `data` must be `null`, as `Nothing` declares.
async fn call_nothing(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) {
    let bytes = send(app, method, uri, body).await;
    let envelope: ApiResponse<Nothing> = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("{method} {uri} is not the declared type: {e}"));
    assert!(envelope.success, "{method} {uri}");
    assert!(envelope.data.is_none(), "{method} {uri}: data is not null");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_routes_with_nothing_to_return_send_a_null_data() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state);
    let person = Uuid::new_v4();

    let who = |p: &str| format!("/api/patients/{person}/{p}");
    call_nothing(
        &app,
        "PUT",
        &who("transport-eligibility"),
        Some(json!({"eligible_by": "qual34", "basis": "mobility"})),
    )
    .await;
    call_nothing(&app, "DELETE", &who("transport-eligibility"), None).await;
    call_nothing(
        &app,
        "PUT",
        &who("transport-consent"),
        Some(json!({"recorded_by": "qual34", "basis": "Art. 6(1)(e)"})),
    )
    .await;
    call_nothing(&app, "DELETE", &who("transport-consent"), None).await;
    call_nothing(
        &app,
        "PUT",
        &who("whatsapp-optin"),
        Some(json!({"source": "portal", "recorded_by": "qual34"})),
    )
    .await;
    call_nothing(&app, "DELETE", &who("whatsapp-optin"), None).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_print_queue_support_and_interpreter_routes_send_the_types_they_declare() {
    use patient_administration_system::api::rest::handlers_interpreter::NeedRecorded;
    use patient_administration_system::api::rest::handlers_print::PrintQueue;
    use patient_administration_system::api::rest::handlers_support::Raised;
    use patient_administration_system::interpreter::UnconfirmedRequest;
    use patient_administration_system::support::SupportFlag;

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state);
    let person = Uuid::new_v4();

    let queue: PrintQueue = call(&app, "GET", "/api/print-queue?limit=5", None).await;
    assert!(queue.waiting >= queue.letters.len() as i64);

    let raised: Raised = call(
        &app,
        "POST",
        "/api/support-flags",
        Some(json!({"person_id": person, "kind": "needs_support_call", "created_by": "qual34"})),
    )
    .await;
    let open: Vec<SupportFlag> = call(&app, "GET", "/api/support-flags/open?limit=100", None).await;
    assert!(open.iter().all(|f| f.id != Uuid::nil()));
    assert_ne!(raised.id, Uuid::nil());

    let need: NeedRecorded = call(
        &app,
        "PUT",
        &format!("/api/patients/{person}/interpreter-need"),
        Some(json!({"kind": "spoken", "language": "pl", "recorded_by": "qual34"})),
    )
    .await;
    assert_eq!(need.person_id, person);
    assert_eq!(need.language.as_deref(), Some("pl"));
    let _: Vec<UnconfirmedRequest> = call(
        &app,
        "GET",
        "/api/interpreter-requests/unconfirmed?limit=5",
        None,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_proxy_and_identity_proofing_routes_send_the_types_they_declare() {
    use patient_administration_system::api::rest::handlers_proxy::Revoked;
    use patient_administration_system::identity::Proofing;
    use patient_administration_system::proxy::Authority;

    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state);
    let person = Uuid::new_v4();
    let proxy = Uuid::new_v4();

    let granted: Authority = call(
        &app,
        "POST",
        &format!("/api/patients/{person}/proxies"),
        Some(json!({"proxy_person_id": proxy, "relationship": "daughter",
                    "basis": "patient_consent", "scopes": ["receive"],
                    "expires_at": chrono::Utc::now() + chrono::Duration::days(30),
                    "recorded_by": "Nurse A"})),
    )
    .await;
    let seen: Vec<Appointment> = call(
        &app,
        "GET",
        &format!("/api/proxy/patients/{person}/appointments?proxy_person_id={proxy}"),
        None,
    )
    .await;
    assert!(seen.is_empty());
    let revoked: Revoked = call(
        &app,
        "POST",
        &format!("/api/proxies/{}/revoke", granted.id),
        Some(json!({"revoked_by": "Nurse A"})),
    )
    .await;
    assert_eq!(revoked.id, granted.id);

    let subject = format!("sub-{}", Uuid::new_v4());
    let url = format!("/api/identity-proofings/{subject}");
    call_nothing(
        &app,
        "PUT",
        &url,
        Some(json!({"person_id": person, "method": "in_person", "proofed_by": "Nurse A"})),
    )
    .await;
    let proofing: Proofing = call(&app, "GET", &url, None).await;
    assert_eq!(proofing.person_id, person);
}
