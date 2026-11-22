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

async fn raw(
    app: &axum::Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let r = app
        .clone()
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let st = r.status();
    let bytes = to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_r4_capability_statement_names_the_server_and_what_it_does() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::fhir::fhir_router(s);
    let (st, v) = raw(
        &app,
        "GET",
        "/fhir/r4/metadata",
        &[("host", "pas.example"), ("x-forwarded-proto", "https")],
        "",
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["resourceType"], "CapabilityStatement");
    assert_eq!(v["fhirVersion"], "4.0.1");
    if std::env::var("PAS_FHIR_BASE_URL").is_err() {
        assert_eq!(v["implementation"]["url"], "https://pas.example/fhir/r4");
    }
    let ops: Vec<&str> = v["rest"][0]["resource"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["operation"].as_array().into_iter().flatten())
        .filter_map(|o| o["name"].as_str())
        .collect();
    assert_eq!(ops, ["find", "hold", "book"]);
    // No Host header: the address falls back to localhost over http.
    let (_, v) = raw(&app, "GET", "/fhir/r4/metadata", &[], "").await;
    if std::env::var("PAS_FHIR_BASE_URL").is_err() {
        assert_eq!(v["implementation"]["url"], "http://localhost/fhir/r4");
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_scheduling_operations_refuse_what_they_cannot_read_with_an_operation_outcome() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::fhir::fhir_router(s);
    let json = [("content-type", "application/fhir+json")];
    let patient = Uuid::new_v4();
    let param = |name: &str, value: serde_json::Value| {
        let mut p = serde_json::json!({ "name": name });
        p.as_object_mut()
            .unwrap()
            .extend(value.as_object().unwrap().clone());
        p
    };
    let patient_ref = param(
        "patient-reference",
        serde_json::json!({ "valueReference": { "reference": format!("Patient/{patient}") } }),
    );
    let body = |params: Vec<serde_json::Value>| {
        serde_json::json!({ "resourceType": "Parameters", "parameter": params }).to_string()
    };
    let find = "/fhir/r4/Appointment/$find";
    let cases: Vec<(String, String)> = vec![
        (find.into(), "not json".into()),
        (find.into(), body(vec![])),
        (
            find.into(),
            body(vec![patient_ref.clone(), patient_ref.clone()]),
        ),
        (
            find.into(),
            body(vec![
                patient_ref.clone(),
                param(
                    "start",
                    serde_json::json!({ "valueDateTime": "next tuesday" }),
                ),
            ]),
        ),
        (
            find.into(),
            body(vec![param(
                "patient-reference",
                serde_json::json!({ "valueReference": { "reference": "Practitioner/x" } }),
            )]),
        ),
        (format!("{find}?_count=0"), body(vec![patient_ref.clone()])),
        (format!("{find}?_count=many"), body(vec![patient_ref])),
        ("/fhir/r4/Appointment/$hold".into(), body(vec![])),
        ("/fhir/r4/Appointment/$book".into(), body(vec![])),
    ];
    for (uri, b) in cases {
        let (st, v) = raw(&app, "POST", &uri, &json, &b).await;
        assert!(st.is_client_error(), "{uri} {b}: {st}");
        if b != "not json" {
            // (A body that is not JSON never reaches the operation: the extractor answers it.)
            assert_eq!(v["resourceType"], "OperationOutcome", "{uri} {b}: {v}");
        }
    }
    // A proposal nobody made.
    let unknown = body(vec![param(
        "appointment-reference",
        serde_json::json!({ "valueReference": { "reference": format!("Appointment/{}", Uuid::new_v4()) } }),
    )]);
    for op in ["$hold", "$book"] {
        let (st, v) = raw(
            &app,
            "POST",
            &format!("/fhir/r4/Appointment/{op}"),
            &json,
            &unknown,
        )
        .await;
        assert!(st.is_client_error(), "{op}: {st} {v}");
    }
}
