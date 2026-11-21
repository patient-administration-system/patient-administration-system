//! The FHIR R4 read endpoints for the Wales exchange (NEW-1): a stored appointment and encounter are read back
//! as FHIR resources with the R4 content type, and an id nobody has is an `OperationOutcome` 404.
//! Needs PostgreSQL: `cargo test -- --ignored`.
mod common;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::upstream::StubPersonClient;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let st = r.status();
    let ct = r
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (st, ct, serde_json::from_slice(&body).unwrap_or_default())
}

async fn post(app: &axum::Router, path: &str, body: String) -> String {
    let r = app
        .clone()
        .oneshot(
            Request::post(path)
                .header("content-type", "application/hl7-v2")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    String::from_utf8(to_bytes(r.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
}

fn after(text: &str, key: &str) -> String {
    let at = text.find(key).unwrap() + key.len();
    text[at..at + 36].to_string()
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn stored_rows_read_back_as_r4_and_unknown_ids_are_404() {
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()));
    let rest = patient_administration_system::api::rest::router(s.clone());
    let app = patient_administration_system::api::fhir::fhir_router(s);
    let tag = Uuid::new_v4().simple().to_string();
    let pid = format!("PID|1||MRN-{tag}^^^HOSP^MR||Lovelace^Ada||19900115|F\r");
    let head = |ev: &str| {
        format!(
            "MSH|^~\\&|EMR-{tag}|WARDS|PAS|HOSP|20261116120000||{ev}|{}|P|2.5\r",
            Uuid::new_v4().simple()
        )
    };
    let reg = post(
        &rest,
        "/api/hl7/v2/register",
        format!("{}{pid}PV1|1|O\r", head("ADT^A04")),
    )
    .await;
    let enc = after(&reg, "encounter=");
    let book = post(
        &rest,
        "/api/hl7/v2/schedule-book",
        format!(
            "{}SCH|P1||||||visit||30|min|^^^20291201093000\r{pid}",
            head("SIU^S12")
        ),
    )
    .await;
    let appt = after(&book, "filler=");

    let (st, ct, v) = get(&app, &format!("/fhir/r4/Appointment/{appt}")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(ct.contains("fhirVersion=4.0"), "{ct}");
    assert_eq!(
        (v["resourceType"].as_str(), v["id"].as_str()),
        (Some("Appointment"), Some(appt.as_str()))
    );

    let (st, ct, v) = get(&app, &format!("/fhir/r4/Encounter/{enc}")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(ct.contains("fhirVersion=4.0"), "{ct}");
    assert_eq!(v["resourceType"], "Encounter");

    for kind in ["Appointment", "Encounter", "ServiceRequest"] {
        let (st, ct, v) = get(&app, &format!("/fhir/r4/{kind}/{}", Uuid::new_v4())).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{kind}");
        assert!(ct.contains("fhirVersion=4.0"), "{kind}: {ct}");
        assert_eq!(v["resourceType"], "OperationOutcome", "{kind}");
        assert_eq!(v["issue"][0]["code"], "not-found", "{kind}");
    }
}
