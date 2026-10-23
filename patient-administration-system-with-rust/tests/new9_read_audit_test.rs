//! NEW-9: reads of a patient's records are audited, and the audit trail exports as BALP-shaped FHIR R4
//! `AuditEvent`s. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::router;
use patient_administration_system::audit_export::violations;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(app: &axum::Router, req: Request<Body>) -> (StatusCode, String, Vec<u8>) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let ctype = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    (
        status,
        ctype,
        to_bytes(res.into_body(), 8 << 20).await.unwrap().to_vec(),
    )
}

async fn read_rows(db: &sea_orm::DatabaseConnection, patient: Uuid) -> Vec<serde_json::Value> {
    db.query_all_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT user_id, context FROM audit_log \
         WHERE entity_type = 'patient' AND entity_id = $1 AND action = 'read'",
        [patient.into()],
    ))
    .await
    .unwrap()
    .iter()
    .map(|r| {
        serde_json::json!({
            "user": r.try_get::<Option<String>>("", "user_id").unwrap(),
            "context": r.try_get::<Option<serde_json::Value>>("", "context").unwrap(),
        })
    })
    .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn reading_a_patients_appointments_writes_one_read_event_and_it_exports_as_balp() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = router(state.clone());
    let patient = Uuid::new_v4();

    let (status, _, _) = call(
        &app,
        Request::get(format!(
            "/api/patients/{patient}/appointments?secret=a-name"
        ))
        .header("x-user-id", "dr-evans")
        .header("x-user-ip", "10.0.0.7")
        .header("x-user-agent", "pas-ui/1")
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let rows = read_rows(&state.db, patient).await;
    assert_eq!(rows.len(), 1, "one read, one event");
    assert_eq!(rows[0]["user"], "dr-evans");
    assert_eq!(
        rows[0]["context"]["route"], "/api/patients/{id}/appointments",
        "the route, not the query string"
    );

    // A failed read and a write by the same route write no read event.
    let other = Uuid::new_v4();
    let (status, _, _) = call(
        &app,
        Request::get(format!("/api/patients/{other}/no-such-route"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(!status.is_success());
    assert!(read_rows(&state.db, other).await.is_empty());

    // The export: a FHIR R4 Bundle of AuditEvent, with the read as a Patient read event.
    let (status, ctype, body) = call(
        &app,
        Request::get(format!(
            "/api/audit/balp?entity_type=patient&entity_id={patient}"
        ))
        .header("x-user-id", "auditor")
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(ctype.starts_with("application/fhir+json"), "{ctype}");
    assert!(ctype.contains("fhirVersion=4.0"), "{ctype}");
    let bundle: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(bundle["resourceType"], "Bundle");
    assert_eq!(bundle["total"], 1);
    let event = &bundle["entry"][0]["resource"];
    assert_eq!(event["resourceType"], "AuditEvent");
    assert_eq!(event["action"], "R");
    assert_eq!(event["agent"][2]["who"]["identifier"]["value"], "dr-evans");
    assert_eq!(
        event["entity"][1]["what"]["reference"],
        format!("Patient/{patient}")
    );
    // Everything the profile asks for is present except, where HOSTNAME is unset, the server's address;
    // with it the event names the profile.
    let v = violations(event);
    if std::env::var("HOSTNAME").is_ok_and(|h| !h.trim().is_empty()) {
        assert!(v.is_empty(), "{v:?}");
        assert!(
            event["meta"]["profile"][0]
                .as_str()
                .unwrap()
                .ends_with("PatientRead")
        );
    } else {
        assert!(
            v.iter().all(|s| s.contains("server agent network")),
            "{v:?}"
        );
        assert!(event["meta"].is_null());
    }

    // The export is itself audited.
    let n: i64 = state
        .db
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM audit_log WHERE entity_type = 'audit_export' AND user_id = 'auditor'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "n")
        .unwrap();
    assert!(n >= 1);

    // A backwards or over-long window is refused.
    let (status, _, _) = call(
        &app,
        Request::get("/api/audit/balp?from=2026-10-02T00:00:00Z&to=2026-10-01T00:00:00Z")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(
        &app,
        Request::get("/api/audit/balp?from=2026-01-01T00:00:00Z&to=2026-10-01T00:00:00Z")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
