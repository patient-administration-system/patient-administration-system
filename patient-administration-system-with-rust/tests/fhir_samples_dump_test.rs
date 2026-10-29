//! Writes real responses of the R4 and R5 routes to files so the official HL7 validator can check them
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

/// The Wales R4 reads: a booked appointment, an encounter and a referral as a ServiceRequest.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn dump_wales_r4_reads_for_the_validator() {
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::models::encounter::{Encounter, EncounterClass};
    use patient_administration_system::referrals::NewReferral;

    let Ok(dir) = std::env::var("PAS_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let state = common::build_state(&common::database_url().unwrap()).await;
    let ctx = UserContext::system();
    let person = Uuid::new_v4();

    let sched = common::new_schedule(&state.db).await;
    let slot = common::free_slot(&state.db, sched.id, 20).await;
    let appt = state
        .scheduling
        .book_slot(slot.id, person, &ctx)
        .await
        .unwrap();

    let mut enc = Encounter::new(person, EncounterClass::Outpatient);
    enc.worker_id = Some(Uuid::new_v4());
    enc.reason = Some("follow-up of knee pain".into());
    let enc = EncounterRepository::create(&state.db, &enc).await.unwrap();

    let (referral, _) = state
        .referrals
        .create(
            NewReferral {
                person_id: person,
                source: "e-rs".into(),
                external_ref: Some(format!("U{}", Uuid::new_v4().simple())),
                target_service: "orthopaedics".into(),
                reason: Some("knee pain for six weeks".into()),
                referring_worker_id: Some(Uuid::new_v4()),
                received_at: None,
            },
            &ctx,
        )
        .await
        .unwrap();

    let app = fhir_router(state.clone());
    for (kind, id) in [
        ("Appointment", appt.id),
        ("Encounter", enc.id),
        ("ServiceRequest", referral.id),
    ] {
        let res = app
            .clone()
            .oneshot(
                Request::get(format!("/fhir/r4/{kind}/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "{kind}");
        let ctype = res.headers()["content-type"].to_str().unwrap().to_string();
        assert!(ctype.contains("fhirVersion=4.0"), "{ctype}");
        let v = body(res).await;
        assert_eq!(v["resourceType"], kind);
        assert_eq!(v["id"], id.to_string());
        write(&dir, &format!("wales-{kind}.json"), &v);
    }
    // An id that is not there is an OperationOutcome, not a crash.
    let res = app
        .oneshot(
            Request::get(format!("/fhir/r4/Appointment/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
    assert_eq!(body(res).await["resourceType"], "OperationOutcome");
}

/// The R5 routes: the CapabilityStatement and a read of each resource type that works by id.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn dump_r5_reads_for_the_validator() {
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::models::encounter::{Encounter, EncounterClass};

    let Ok(dir) = std::env::var("PAS_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let state = common::build_state(&common::database_url().unwrap()).await;
    let ctx = UserContext::system();
    let person = Uuid::new_v4();

    let sched = common::new_schedule(&state.db).await;
    let slot = common::free_slot(&state.db, sched.id, 20).await;
    let appt = state
        .scheduling
        .book_slot(slot.id, person, &ctx)
        .await
        .unwrap();
    let mut enc = Encounter::new(person, EncounterClass::Outpatient);
    enc.worker_id = Some(Uuid::new_v4());
    enc.reason = Some("follow-up of knee pain".into());
    let enc = EncounterRepository::create(&state.db, &enc).await.unwrap();

    let app = fhir_router(state.clone());
    let res = app
        .clone()
        .oneshot(Request::get("/fhir/metadata").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(res.status().is_success());
    write(&dir, "r5-metadata.json", &body(res).await);

    for (kind, id) in [
        ("Appointment", appt.id),
        ("Encounter", enc.id),
        ("Slot", slot.id),
        ("Schedule", sched.id),
    ] {
        let res = app
            .clone()
            .oneshot(
                Request::get(format!("/fhir/{kind}/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "{kind}");
        let ctype = res.headers()["content-type"].to_str().unwrap().to_string();
        assert!(ctype.contains("fhirVersion=5.0"), "{ctype}");
        let v = body(res).await;
        assert_eq!(v["resourceType"], kind);
        assert_eq!(v["id"], id.to_string());
        write(&dir, &format!("r5-{kind}.json"), &v);
    }
}

/// The other R5 reads (QUAL-39): a bed as a Location, a coverage, a flag and a triage Observation search, a
/// Patient and a Practitioner through stub upstreams.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn dump_the_other_r5_reads_for_the_validator() {
    use patient_administration_system::db::repositories::encounter::EncounterRepository;
    use patient_administration_system::models::encounter::{Encounter, EncounterClass};

    let Ok(dir) = std::env::var("PAS_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let state = common::build_state(&common::database_url().unwrap()).await;
    let ctx = UserContext::system();
    let person = Uuid::new_v4();
    use patient_administration_system::adjustments::{AdjustmentService, NewAdjustment};
    use patient_administration_system::db::repositories::bed::BedRepository;
    use patient_administration_system::db::repositories::coverage::CoverageRepository;
    use patient_administration_system::db::repositories::facility::{
        FacilityRepository, RoomRepository, WardRepository,
    };
    use patient_administration_system::models::Address;
    use patient_administration_system::models::coverage::Coverage;
    use patient_administration_system::models::facility::{Bed, Facility, Room, Ward};
    use patient_administration_system::triage::TriageService;
    use patient_administration_system::upstream::{
        Person, StubPersonClient, StubWorkerClient, Worker,
    };

    let unique = Uuid::new_v4().simple().to_string();
    let facility = Facility::new("F".into(), format!("F{unique}"), Address::default());
    FacilityRepository::create(&state.db, &facility)
        .await
        .unwrap();
    let ward = Ward::new(facility.id, "W".into(), format!("W{unique}"), 4);
    WardRepository::create(&state.db, &ward).await.unwrap();
    let room = Room::new(ward.id, "R".into(), format!("R{unique}"));
    RoomRepository::create(&state.db, &room).await.unwrap();
    let bed = Bed::new(room.id, "B1".into(), format!("B{unique}"));
    let bed = BedRepository::create(&state.db, &bed).await.unwrap();
    let mut cover = Coverage::new(person, "Self-pay", "P-1");
    cover.payor_identifier = Some("PAYER-9".into());
    let cover = CoverageRepository::create(&state.db, &cover).await.unwrap();

    AdjustmentService::new(state.db.clone())
        .record(
            person,
            &NewAdjustment {
                code: "easy_read",
                note: Some("prefers the front entrance"),
                review_by: None,
                recorded_by: "Nurse Okafor",
            },
            Utc::now(),
            &ctx,
        )
        .await
        .unwrap();
    let er = EncounterRepository::create(
        &state.db,
        &Encounter::new(person, EncounterClass::Emergency),
    )
    .await
    .unwrap();
    let now = Utc::now();
    TriageService::new(state.db.clone())
        .record(
            er.id,
            "mts",
            Some("3.8"),
            "urgent",
            None,
            "Nurse Okafor",
            now - Duration::minutes(5),
            now,
            &ctx,
        )
        .await
        .unwrap();

    let people = StubPersonClient::new();
    let patient_id = Uuid::new_v4();
    people.insert(Person {
        id: patient_id,
        data: json!({
            "name": { "family": "Doe", "given": ["Jane"] },
            "gender": "female",
            "birth_date": "1990-01-15",
            "identifiers": [{ "value": "MRN-99" }],
            "telecom": [{ "system": "phone", "value": "+44 20 7946 0000" }],
            "active": true
        })
        .as_object()
        .unwrap()
        .clone(),
    });
    let workers = StubWorkerClient::new();
    let worker_id = Uuid::new_v4();
    workers.insert(Worker {
        id: worker_id,
        data: json!({ "name": { "family": "Evans", "given": ["Rhys"] }, "active": true })
            .as_object()
            .unwrap()
            .clone(),
    });
    let app = fhir_router(
        state
            .clone()
            .with_person_client(std::sync::Arc::new(people))
            .with_worker_client(std::sync::Arc::new(workers)),
    );
    for (name, uri) in [
        ("Location", format!("/fhir/Location/{}", bed.id)),
        ("Coverage", format!("/fhir/Coverage/{}", cover.id)),
        ("Flag", format!("/fhir/Flag?patient=Patient/{person}")),
        (
            "Observation",
            format!("/fhir/Observation?encounter=Encounter/{}", er.id),
        ),
        ("Patient", format!("/fhir/Patient/{patient_id}")),
        ("Practitioner", format!("/fhir/Practitioner/{worker_id}")),
    ] {
        let res = app
            .clone()
            .oneshot(Request::get(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(res.status().is_success(), "{name}: {}", res.status());
        write(&dir, &format!("r5-{name}.json"), &body(res).await);
    }
    // The error replies are OperationOutcomes: a read of nothing, and a Bundle post, which refuses every entry
    // today (Patient and Practitioner belong to their services; the rest have no creator yet).
    let res = app
        .clone()
        .oneshot(
            Request::get(format!("/fhir/Slot/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
    write(&dir, "r5-OperationOutcome-not-found.json", &body(res).await);
    let res = app
        .oneshot(
            Request::post("/fhir")
                .header("content-type", "application/fhir+json")
                .body(Body::from(
                    json!({ "resourceType": "Bundle", "type": "transaction", "entry": [
                        { "resource": { "resourceType": "Patient" } },
                        { "resource": { "resourceType": "Encounter" } },
                    ] })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    write(&dir, "r5-OperationOutcome-bundle.json", &body(res).await);
}
