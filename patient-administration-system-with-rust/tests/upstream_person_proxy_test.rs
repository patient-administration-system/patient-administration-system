//! Integration tests for the v0.44.0 Phase 3a `/api/patients/*` proxy
//! over `crate::upstream::PersonClient`. Uses the in-memory
//! `StubPersonClient` so the tests work offline.
//!
//! Gated on `DATABASE_URL` only because `AppState::new` requires a real
//! `DatabaseConnection` for its non-proxy services. The proxy code path
//! itself does not query the DB.

mod common;

use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::upstream::{Person, StubPersonClient};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn person_with_id(id: Uuid, family: &str) -> Person {
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        serde_json::json!({ "family": family, "given": ["Jane"] }),
    );
    data.insert("active".into(), serde_json::json!(true));
    Person { id, data }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_get_patient_returns_seeded_person() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubPersonClient::new());
    let id = Uuid::new_v4();
    stub.insert(person_with_id(id, "Doe"));

    let state = common::build_state(&url).await.with_person_client(stub);
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/patients/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["success"], Value::Bool(true));
    assert_eq!(env["data"]["id"], serde_json::json!(id));
    assert_eq!(env["data"]["name"]["family"], serde_json::json!("Doe"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_get_unknown_patient_returns_404() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubPersonClient::new());
    let state = common::build_state(&url).await.with_person_client(stub);
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/patients/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let env = body_json(resp).await;
    assert_eq!(env["code"], serde_json::json!("not_found"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_create_patient_returns_new_id() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubPersonClient::new());
    let state = common::build_state(&url)
        .await
        .with_person_client(stub.clone());
    let app = patient_administration_system::api::rest::router(state);

    let body = serde_json::json!({
        "name": { "family": "Smith", "given": ["Pat"] },
        "active": true,
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/patients")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert!(
        env["data"]["id"].is_string(),
        "id should be present, got {env:?}"
    );
    assert_eq!(stub.len(), 1, "stub should have one person");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_delete_patient_removes_from_stub() {
    let Some(url) = common::database_url() else {
        return;
    };
    let stub = Arc::new(StubPersonClient::new());
    let id = Uuid::new_v4();
    stub.insert(person_with_id(id, "Doe"));
    let state = common::build_state(&url)
        .await
        .with_person_client(stub.clone());
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/patients/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(stub.is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_proxy_returns_503_when_upstream_client_unset() {
    let Some(url) = common::database_url() else {
        return;
    };
    // No with_person_client call → AppState.person_client is None.
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/patients/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let env = body_json(resp).await;
    assert_eq!(env["code"], serde_json::json!("upstream_unavailable"));
}

/// OFH-2: a bulk export of patients is audited as one `patient_search` read, with the records it held when
/// the format makes the count plain (JSON here) and no count for XML, whose shape is the person-service's.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patient_export_is_audited_with_its_count() {
    use patient_administration_system::db::row::RowExt;
    use patient_administration_system::db::sql::{Conn, Stmt};
    let url = common::database_url().unwrap();
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(person_with_id(Uuid::new_v4(), "Doe"));
    stub.insert(person_with_id(Uuid::new_v4(), "Roe"));
    let state = common::build_state(&url).await.with_person_client(stub);
    let app = patient_administration_system::api::rest::router(state.clone());

    let user = format!("exporter-{}", Uuid::new_v4());
    for format in ["json", "xml"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/patients/export.{format}"))
                    .header("x-user-id", &user)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{format}");
    }
    let rows: Vec<Value> = state
        .db
        .all(Stmt::new(
            "SELECT context::text AS c FROM audit_log \
             WHERE entity_type = 'patient_search' AND action = 'read' AND user_id = $1 ORDER BY at",
            [user.into()],
        ))
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_str(&r.col::<String>("c").unwrap()).unwrap())
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0]["route"], "/api/patients/export.json");
    assert_eq!(rows[0]["returned"], 2);
    assert_eq!(rows[1]["route"], "/api/patients/export.xml");
    assert!(rows[1]["returned"].is_null(), "{rows:?}");
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    (resp.status(), body_json(resp).await)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn update_search_masked_export_audit_and_import_go_through_the_client() {
    let url = common::database_url().unwrap();
    let stub = Arc::new(StubPersonClient::new());
    let id = Uuid::new_v4();
    stub.insert(person_with_id(id, "Doe"));
    let state = common::build_state(&url).await.with_person_client(stub);
    let app = patient_administration_system::api::rest::router(state);

    let (st, v) = call(
        &app,
        "PUT",
        &format!("/api/patients/{id}"),
        Some(serde_json::json!({ "name": { "family": "Roe" }, "active": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["name"]["family"], "Roe");
    // Only an object can update a person; a bad identifier is refused before the upstream hears of it.
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/api/patients/{id}"),
        Some(serde_json::json!([1])),
    )
    .await;
    assert!(st.is_client_error());
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/api/patients/{id}"),
        Some(serde_json::json!({ "identifiers": [{ "system": "https://fhir.nhs.uk/Id/nhs-number", "value": "1234567890" }] })),
    )
    .await;
    assert!(st.is_client_error(), "an NHS number that fails its check");
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/api/patients/{}", Uuid::new_v4()),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, v) = call(&app, "GET", "/api/patients?q=Roe&limit=5", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["data"].is_array());
    let (st, _) = call(&app, "GET", "/api/patients", None).await;
    assert_eq!(st, StatusCode::OK, "search with no query");

    let (st, v) = call(&app, "GET", &format!("/api/patients/{id}/masked"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = call(&app, "GET", &format!("/api/patients/{id}/export"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = call(
        &app,
        "GET",
        &format!("/api/patients/{id}/audit?limit=5"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    for tail in ["masked", "export", "audit"] {
        let (st, _) = call(
            &app,
            "GET",
            &format!("/api/patients/{}/{tail}", Uuid::new_v4()),
            None,
        )
        .await;
        assert!(st.is_client_error() || tail == "audit", "{tail}: {st}");
    }

    for format in ["tsv", "csv", "json", "xml"] {
        let (st, _) = call(&app, "GET", &format!("/api/patients/export.{format}"), None).await;
        assert_eq!(st, StatusCode::OK, "{format}");
    }
    let (st, v) = call(
        &app,
        "POST",
        "/api/patients/import",
        Some(serde_json::json!([])),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_patient_route_says_so_when_there_is_no_person_service() {
    let url = common::database_url().unwrap();
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);
    let id = Uuid::new_v4();
    for (method, uri, body) in [
        (
            "POST",
            "/api/patients".to_string(),
            Some(serde_json::json!({})),
        ),
        (
            "PUT",
            format!("/api/patients/{id}"),
            Some(serde_json::json!({})),
        ),
        ("DELETE", format!("/api/patients/{id}"), None),
        ("GET", "/api/patients?q=x".to_string(), None),
        ("GET", format!("/api/patients/{id}/masked"), None),
        ("GET", format!("/api/patients/{id}/export"), None),
        ("GET", format!("/api/patients/{id}/audit"), None),
        ("GET", "/api/patients/export.json".to_string(), None),
        ("GET", "/api/patients/export.csv".to_string(), None),
        (
            "POST",
            "/api/patients/import".to_string(),
            Some(serde_json::json!([])),
        ),
    ] {
        let (st, _) = call(&app, method, &uri, body).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{method} {uri}");
    }
}
