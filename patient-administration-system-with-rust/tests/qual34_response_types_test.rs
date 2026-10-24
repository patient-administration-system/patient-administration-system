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
