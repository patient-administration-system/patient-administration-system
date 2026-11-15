//! DTAC-8, QUAL-40: an identifier whose system has a scheme (here the NHS number, one scheme among any) is
//! checked before a patient body reaches person-service; an identifier of any other system is not.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::upstream::StubPersonClient;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_endpoint_validates_and_the_patient_proxy_refuses_an_identifier_that_fails_its_scheme()
{
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()));
    let app = patient_administration_system::api::rest::router(s);
    let call = |method: &'static str, uri: &'static str, body: serde_json::Value| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
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
    let (st, v) = call(
        "POST",
        "/api/validate/identifier",
        serde_json::json!({ "system": "https://fhir.nhs.uk/Id/nhs-number", "value": "943 476 5919" }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["formatted"], "943 476 5919");
    let (st, _) = call(
        "POST",
        "/api/validate/identifier",
        serde_json::json!({ "system": "https://fhir.nhs.uk/Id/nhs-number", "value": "9434765918" }),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // A system with no scheme is refused, not called valid.
    let (st, _) = call(
        "POST",
        "/api/validate/identifier",
        serde_json::json!({ "system": "https://example.test/mrn", "value": "A-1" }),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let bad = serde_json::json!({ "name": { "family": "Lovelace" }, "identifiers": [{ "system": "https://fhir.nhs.uk/Id/nhs-number", "value": "9434765918" }] });
    let (st, v) = call("POST", "/api/patients", bad.clone()).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["detail"].as_str().unwrap().contains("NHS number"));
    let (st, _) = call(
        "PUT",
        "/api/patients/00000000-0000-0000-0000-000000000001",
        bad,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let good = serde_json::json!({ "name": { "family": "Lovelace" }, "identifiers": [{ "system": "https://fhir.nhs.uk/Id/nhs-number", "value": "9434765919" }] });
    let (st, v) = call("POST", "/api/patients", good).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // An identifier of a system with no scheme passes untouched.
    let other = serde_json::json!({ "name": { "family": "Lovelace" }, "identifiers": [{ "system": "https://example.test/mrn", "value": "A-1" }] });
    let (st, _) = call("POST", "/api/patients", other).await;
    assert_eq!(st, StatusCode::OK);
}
