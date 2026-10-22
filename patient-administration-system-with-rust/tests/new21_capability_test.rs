//! NEW-21: the FHIR CapabilityStatement claims nothing the server does not do. Needs PostgreSQL:
//! `cargo test -- --ignored`.
//!
//! Every interaction in `capability::SUPPORTED` is sent to the FHIR router. It may succeed or refuse the
//! request as invalid or not found, but it must reach a handler: a 503, a 405 or the router's own empty
//! 404 means the statement promises an interaction that is not there. The upstream services are stubs, so
//! Patient and Practitioner reach their handlers rather than "person-service not configured".

mod common;

use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::fhir::{capability, fhir_router};
use patient_administration_system::upstream::{StubPersonClient, StubWorkerClient};
use tower::ServiceExt;
use uuid::Uuid;

async fn send(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/fhir+json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_interaction_the_statement_lists_reaches_a_handler() {
    let state = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()))
        .with_worker_client(Arc::new(StubWorkerClient::new()));
    let app = fhir_router(state);
    let mut broken = Vec::new();
    for s in capability::SUPPORTED {
        let t = s.resource;
        let id = Uuid::new_v4();
        for interaction in s.interactions {
            let (method, uri, body) = match *interaction {
                "read" => ("GET", format!("/fhir/{t}/{id}"), None),
                "update" => (
                    "PUT",
                    format!("/fhir/{t}/{id}"),
                    Some(serde_json::json!({ "resourceType": t, "id": id })),
                ),
                "delete" => ("DELETE", format!("/fhir/{t}/{id}"), None),
                "create" => (
                    "POST",
                    format!("/fhir/{t}"),
                    Some(serde_json::json!({ "resourceType": t })),
                ),
                "search-type" => ("GET", format!("/fhir/{t}"), None),
                other => panic!("no request written for the interaction {other:?}"),
            };
            let (status, text) = send(app.clone(), method, &uri, body).await;
            let unrouted = status == StatusCode::NOT_FOUND && text.trim().is_empty();
            if status == StatusCode::SERVICE_UNAVAILABLE
                || status == StatusCode::METHOD_NOT_ALLOWED
                || status == StatusCode::NOT_IMPLEMENTED
                || unrouted
            {
                broken.push(format!(
                    "{t} {interaction}: {method} {uri} answered {status} {text}"
                ));
            }
        }
    }
    assert!(broken.is_empty(), "{broken:#?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_metadata_is_the_statement_and_the_removed_stubs_are_gone() {
    let app = fhir_router(common::build_state(&common::database_url().unwrap()).await);
    let (status, text) = send(app.clone(), "GET", "/fhir/metadata", None).await;
    assert_eq!(status, StatusCode::OK);
    let served: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(served, capability::statement());

    // Creating an Appointment or Encounter through FHIR was a 503 that pointed elsewhere; it is now simply not
    // a route, and the statement does not list it.
    for t in ["Appointment", "Encounter"] {
        let (status, _) = send(
            app.clone(),
            "POST",
            &format!("/fhir/{t}"),
            Some(serde_json::json!({ "resourceType": t })),
        )
        .await;
        assert!(
            matches!(
                status,
                StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_FOUND
            ),
            "POST /fhir/{t}: {status}"
        );
        let listed = capability::SUPPORTED
            .iter()
            .any(|s| s.resource == t && s.interactions.contains(&"create"));
        assert!(!listed, "{t} create is not supported");
    }
}
