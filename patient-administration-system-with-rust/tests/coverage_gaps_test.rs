//! Paths that no other test reaches: the row of a referral read and written as a model, and a background job that
//! is skipped or panics. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use patient_administration_system::db::entities::referral;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::consent::ConsentRepository;
use patient_administration_system::db::repositories::coverage::CoverageRepository;
use patient_administration_system::db::repositories::waitlist::WaitlistRepository;
use patient_administration_system::db::sql::{Conn, FromRow, Stmt, Table};
use patient_administration_system::jobs::{Outcome, run_exclusive};
use patient_administration_system::models::consent::{Consent, ConsentType};
use patient_administration_system::models::coverage::Coverage;
use patient_administration_system::models::waitlist::{Priority, WaitlistEntry};
use patient_administration_system::upstream::StubPersonClient;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_referral_row_round_trips_as_a_model() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let now = jiff::Timestamp::now();
    let model = referral::Model {
        id: Uuid::new_v4(),
        person_id: Uuid::new_v4(),
        referring_worker_id: Some(Uuid::new_v4()),
        target_service: format!("svc-{}", Uuid::new_v4()),
        reason: Some("cough".into()),
        received_at: now,
        created_at: now,
    };
    assert_eq!(model.id(), model.id);
    assert_eq!(model.values().len(), referral::Model::COLUMNS.len());
    let written = model.insert(&state.db).await.unwrap();
    let read = state
        .db
        .all_as::<referral::Model>(Stmt::new(
            "SELECT * FROM referrals WHERE id = $1",
            [model.id.into()],
        ))
        .await
        .unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].target_service, written.target_service);
    assert_eq!(read[0].referring_worker_id, model.referring_worker_id);
    let _ = state
        .db
        .run(Stmt::new(
            "DELETE FROM referrals WHERE id = $1",
            [model.id.into()],
        ))
        .await;
    let _ = <referral::Model as FromRow>::from_row;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_job_held_elsewhere_is_skipped_and_one_that_panics_is_reported() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let name = format!("coverage-{}", Uuid::new_v4());

    assert!(matches!(
        run_exclusive(&state.db, &name, || async { Ok(()) }).await,
        Outcome::Ran
    ));
    assert!(matches!(
        run_exclusive(&state.db, &name, || async { panic!("a job that fails") }).await,
        Outcome::Panicked
    ));

    // Another replica holds the job's lease while it is asked to run: skipped; once the lease has lapsed it runs.
    state
        .db
        .run(Stmt::new(
            "UPDATE job_leases SET holder = $2, held_until = now() + interval '1 minute' WHERE name = $1",
            [name.clone().into(), Uuid::new_v4().into()],
        ))
        .await
        .unwrap();
    assert!(matches!(
        run_exclusive(&state.db, &name, || async { Ok(()) }).await,
        Outcome::Skipped
    ));
    state
        .db
        .run(Stmt::new(
            "UPDATE job_leases SET held_until = now() - interval '1 second' WHERE name = $1",
            [name.clone().into()],
        ))
        .await
        .unwrap();
    assert!(matches!(
        run_exclusive(&state.db, &name, || async { Ok(()) }).await,
        Outcome::Ran
    ));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_repository_reads_no_route_uses_still_work() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let person = Uuid::new_v4();

    let consent = ConsentRepository::create(
        &state.db,
        &Consent::new(person, ConsentType::Research, jiff::civil::date(2026, 1, 1)),
    )
    .await
    .unwrap();
    let found = ConsentRepository::find_by_id(&state.db, consent.id)
        .await
        .unwrap()
        .expect("the consent just made");
    assert_eq!(found.consent_type, ConsentType::Research);
    assert!(
        ConsentRepository::find_by_id(&state.db, Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );

    let coverage = CoverageRepository::create(&state.db, &Coverage::new(person, "Payer", "P-1"))
        .await
        .unwrap();
    let detached = CoverageRepository::link_to_account(&state.db, coverage.id, None)
        .await
        .unwrap();
    assert_eq!(detached.account_id, None);
    assert!(
        CoverageRepository::link_to_account(&state.db, Uuid::new_v4(), None)
            .await
            .is_err(),
        "a coverage that is not there"
    );

    let service = format!("svc-{}", Uuid::new_v4());
    let entry = WaitlistRepository::create_entry(
        &state.db,
        &WaitlistEntry::new(person, &service, Priority::TwoWeekWait),
    )
    .await
    .unwrap();
    let urgent = WaitlistRepository::list_by_priority(&state.db, Priority::TwoWeekWait)
        .await
        .unwrap();
    assert!(urgent.iter().any(|e| e.id == entry.id));
    assert!(
        WaitlistRepository::list_breaches(&state.db, jiff::Timestamp::now(), 18)
            .await
            .unwrap()
            .is_empty()
    );
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            request = request.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_schedule_sees_each_appointments_adjustments_and_the_bands_refresh_from_person_service()
{
    // A fixed clock, as a test that must not depend on when it runs would use.
    let state = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()))
        .with_clock(patient_administration_system::clock::FixedClock::at(
            jiff::Timestamp::now(),
        ));
    let schedule = common::new_schedule(&state.db).await;
    let slot = common::free_slot(&state.db, schedule.id, 3).await;
    let person = Uuid::new_v4();
    let appointment = state
        .scheduling
        .book_slot(slot.id, person, &UserContext::system())
        .await
        .unwrap();
    let app = patient_administration_system::api::rest::router(state);

    let (_, codes) = call(&app, "GET", "/api/reasonable-adjustments/codes", None).await;
    let codes: Vec<String> = codes["data"]["codes"]
        .as_array()
        .unwrap()
        .iter()
        .take(2)
        .map(|c| c["code"].as_str().unwrap().to_string())
        .collect();
    for code in &codes {
        let (status, body) = call(
            &app,
            "POST",
            &format!("/api/patients/{person}/reasonable-adjustments"),
            Some(serde_json::json!({ "code": code, "recorded_by": "Receptionist" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = call(
        &app,
        "GET",
        &format!(
            "/api/reasonable-adjustments/for-appointments?ids={},{}",
            appointment.id,
            Uuid::new_v4()
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["data"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        1,
        "only the appointment whose patient has some: {body}"
    );
    assert_eq!(
        rows[0]["adjustments"].as_array().unwrap().len(),
        codes.len(),
        "{body}"
    );

    let (status, body) = call(
        &app,
        "POST",
        "/api/analytics/demographics/refresh?limit=5",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A bed with its facility, ward and room, all with fresh codes.
async fn a_bed(db: &patient_administration_system::db::sql::Pool) -> Uuid {
    let tag = Uuid::new_v4().simple().to_string()[..10].to_string();
    let (facility, ward, room, bed) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    for (sql, binds) in [
        (
            "INSERT INTO facilities (id, name, code, address, active, created_at, updated_at) VALUES ($1,$2,$2,'{}'::jsonb,true,now(),now())",
            vec![facility.into(), format!("F{tag}").into()],
        ),
        (
            "INSERT INTO wards (id, facility_id, name, code, capacity, active, created_at, updated_at) VALUES ($1,$2,$3,$3,2,true,now(),now())",
            vec![ward.into(), facility.into(), format!("W{tag}").into()],
        ),
        (
            "INSERT INTO rooms (id, ward_id, name, code, active, created_at, updated_at) VALUES ($1,$2,'r',$3,true,now(),now())",
            vec![room.into(), ward.into(), format!("R{tag}").into()],
        ),
        (
            "INSERT INTO beds (id, room_id, name, code, status, created_at, updated_at) VALUES ($1,$2,'b',$3,'available',now(),now())",
            vec![bed.into(), room.into(), format!("B{tag}").into()],
        ),
    ] {
        db.run(Stmt::new(sql, binds)).await.unwrap();
    }
    bed
}

/// A discharge is cancelled only while the bed is free and there is a discharge to cancel; each refusal
/// changes nothing.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_discharge_is_cancelled_only_into_a_free_bed_and_only_when_there_is_one() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let ctx = UserContext::system();
    let bed = a_bed(&state.db).await;
    let first = state.adt.admit(Uuid::new_v4(), bed, &ctx).await.unwrap();

    // Still in the bed: there is no discharge, and the bed is taken.
    let e = state
        .adt
        .cancel_discharge(first.admission.id, &ctx)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("cannot reinstate"), "{e}");

    // The bed is free but the admission was never discharged.
    state
        .db
        .run(Stmt::new(
            "UPDATE beds SET status = 'available' WHERE id = $1",
            [bed.into()],
        ))
        .await
        .unwrap();
    let e = state
        .adt
        .cancel_discharge(first.admission.id, &ctx)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("no discharge to cancel"), "{e}");
    state
        .db
        .run(Stmt::new(
            "UPDATE beds SET status = 'occupied' WHERE id = $1",
            [bed.into()],
        ))
        .await
        .unwrap();

    // Discharged, and someone else now has the bed.
    state.adt.discharge(first.admission.id, &ctx).await.unwrap();
    state
        .db
        .run(Stmt::new(
            "UPDATE beds SET status = 'available' WHERE id = $1",
            [bed.into()],
        ))
        .await
        .unwrap();
    state.adt.admit(Uuid::new_v4(), bed, &ctx).await.unwrap();
    let e = state
        .adt
        .cancel_discharge(first.admission.id, &ctx)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("cannot reinstate"), "{e}");
}
