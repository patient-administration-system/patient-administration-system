//! Writes real responses of the R4 routes to files so the official HL7 validator can check them
//! (`scripts/validate-fhir.sh`). Does nothing unless `PAS_DUMP_DIR` names a directory. Needs PostgreSQL:
//! `PAS_DUMP_DIR=/some/dir HOSTNAME=pas cargo test --test fhir_samples_dump_test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use chrono::{Datelike, Duration, TimeZone, Utc};
use patient_administration_system::api::fhir::fhir_router;
use patient_administration_system::api::rest::router;
use patient_administration_system::db::repositories::audit::{AuditLogRepository, UserContext};
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::schedule::{ScheduleOwner, Slot};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn body(res: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(res.into_body(), 1 << 22).await.unwrap()).unwrap()
}

fn write(dir: &std::path::Path, name: &str, v: &Value) {
    std::fs::write(dir.join(name), serde_json::to_string_pretty(v).unwrap()).unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn dump_r4_samples_for_the_validator() {
    let Ok(dir) = std::env::var("PAS_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let state = common::build_state(&common::database_url().unwrap()).await;

    // BALP: one event of each action for one patient.
    let app = router(state.clone());
    let patient = Uuid::new_v4();
    let ctx = UserContext {
        user_id: Some("dr-evans".into()),
        user_ip: Some("10.0.0.7".into()),
        user_agent: Some("pas-ui/1".into()),
    };
    let res = app
        .clone()
        .oneshot(
            Request::get(format!("/api/patients/{patient}/appointments"))
                .header("x-user-id", "dr-evans")
                .header("x-user-ip", "10.0.0.7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res.status().is_success());
    AuditLogRepository::log(
        &state.db,
        "patient",
        patient,
        "create",
        None,
        Some(json!({"a": 1})),
        &ctx,
    )
    .await
    .unwrap();
    AuditLogRepository::log(
        &state.db,
        "patient",
        patient,
        "update",
        Some(json!({"a": 1})),
        Some(json!({"a": 2})),
        &ctx,
    )
    .await
    .unwrap();
    AuditLogRepository::log(
        &state.db,
        "patient",
        patient,
        "delete",
        Some(json!({"a": 2})),
        None,
        &ctx,
    )
    .await
    .unwrap();
    let res = app
        .oneshot(
            Request::get(format!(
                "/api/audit/balp?entity_type=patient&entity_id={patient}"
            ))
            .header("x-user-id", "auditor")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let bundle = body(res).await;
    for (i, e) in bundle["entry"].as_array().unwrap().iter().enumerate() {
        let action = e["resource"]["action"].as_str().unwrap().to_string();
        // With the server's name known and a client address, every event claims its Patient profile.
        let claimed = e["resource"]["meta"]["profile"][0]
            .as_str()
            .unwrap_or_default();
        let want = match action.as_str() {
            "R" => "PatientRead",
            "C" => "PatientCreate",
            "U" => "PatientUpdate",
            _ => "PatientDelete",
        };
        assert!(claimed.ends_with(want), "{action}: claimed {claimed:?}");
        write(&dir, &format!("balp-{i}-{action}.json"), &e["resource"]);
    }

    // IHE ITI Scheduling: $find, $hold, $book and the CapabilityStatement.
    let app = fhir_router(state.clone());
    let sched = common::new_schedule(&state.db).await;
    let ScheduleOwner::Practitioner(doctor) = sched.owner else {
        panic!("a practitioner schedule");
    };
    let day = (Utc::now() + Duration::days(30)).date_naive();
    let at = |h| {
        Utc.with_ymd_and_hms(day.year(), day.month(), day.day(), h, 0, 0)
            .unwrap()
    };
    for h in [9, 10] {
        SlotRepository::create(
            &state.db,
            &Slot::new(sched.id, at(h), at(h) + Duration::minutes(15)),
        )
        .await
        .unwrap();
    }
    let post = |op: &'static str, v: Value| {
        let app = app.clone();
        async move {
            let res = app
                .oneshot(
                    Request::post(format!("/fhir/r4/Appointment/{op}"))
                        .header("content-type", "application/fhir+json")
                        .body(Body::from(v.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            body(res).await
        }
    };
    let found = post(
        "$find",
        json!({"resourceType": "Parameters", "parameter": [
            {"name": "start", "valueDateTime": at(0)},
            {"name": "end", "valueDateTime": at(0) + Duration::days(1)},
            {"name": "practitioner", "valueReference": {"reference": format!("Practitioner/{doctor}")}},
            {"name": "patient-reference", "valueReference": {"reference": format!("Patient/{}", Uuid::new_v4())}},
        ]}),
    )
    .await;
    write(&dir, "ihe-find.json", &found);
    let id = found["entry"][0]["resource"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let reference = json!({"resourceType": "Parameters", "parameter": [
        {"name": "appointment-reference", "valueReference": {"reference": format!("Appointment/{id}")}},
    ]});
    write(
        &dir,
        "ihe-hold.json",
        &post("$hold", reference.clone()).await,
    );
    write(&dir, "ihe-book.json", &post("$book", reference).await);
    let res = app
        .oneshot(
            Request::get("/fhir/r4/metadata")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    write(&dir, "ihe-metadata.json", &body(res).await);
}
