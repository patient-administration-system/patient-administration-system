//! QUAL-34: the `data` type each route declares in the OpenAPI document is what the route sends. For each
//! typed route the response body is read back into the declared Rust type through `ApiResponse<T>`, so a
//! field renamed or dropped on either side fails here. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::ApiResponse;
use patient_administration_system::api::rest::router;
use patient_administration_system::models::appointment::Appointment;
use patient_administration_system::models::rtt::{RTTClockEvent, RTTPathway};
use patient_administration_system::models::waitlist::WaitlistEntry;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn call<T: DeserializeOwned>(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> T {
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
