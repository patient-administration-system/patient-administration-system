//! CORE-1: referral management. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::schedule::Slot;
use patient_administration_system::models::waitlist::Priority;
use patient_administration_system::referrals::NewReferral;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    service: String,
    schedule: Uuid,
}

async fn world() -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    W {
        s,
        service,
        schedule,
    }
}

fn new(w: &W, person: Uuid, ubrn: Option<&str>) -> NewReferral {
    NewReferral {
        person_id: person,
        source: "e-rs".into(),
        external_ref: ubrn.map(String::from),
        target_service: w.service.clone(),
        reason: Some("chest pain".into()),
        referring_worker_id: None,
        received_at: None,
    }
}

async fn slot_in(w: &W, days: i64) -> Uuid {
    common::free_slot(&w.s.db, w.schedule, days).await.id
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn receiving_the_same_reference_twice_is_one_referral() {
    let w = world().await;
    let (person, ubrn) = (Uuid::new_v4(), format!("UBRN{}", Uuid::new_v4().simple()));
    let (a, dup) =
        w.s.referrals
            .create(new(&w, person, Some(&ubrn)), &sys())
            .await
            .unwrap();
    assert!(!dup);
    assert_eq!((a.status.as_str(), a.source.as_str()), ("received", "e-rs"));
    let (b, dup) =
        w.s.referrals
            .create(new(&w, person, Some(&ubrn)), &sys())
            .await
            .unwrap();
    assert!(dup);
    assert_eq!(a.id, b.id);
    // The same reference for another patient is a conflict, not a silent merge.
    assert!(
        w.s.referrals
            .create(new(&w, Uuid::new_v4(), Some(&ubrn)), &sys())
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_referral_must_be_well_formed() {
    let w = world().await;
    let p = Uuid::new_v4();
    let mut bad = new(&w, p, Some("U1"));
    bad.source = "fax".into();
    assert!(w.s.referrals.create(bad, &sys()).await.is_err());
    assert!(
        w.s.referrals
            .create(new(&w, p, None), &sys())
            .await
            .is_err(),
        "e-RS needs its UBRN"
    );
    let mut blank = new(&w, p, Some("U2"));
    blank.target_service = "  ".into();
    assert!(w.s.referrals.create(blank, &sys()).await.is_err());
    let mut gp = new(&w, p, None);
    gp.source = "gp".into();
    w.s.referrals
        .create(gp, &sys())
        .await
        .expect("a GP referral may have no reference");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_earliest_free_slot_of_the_service_is_booked_and_linked() {
    let w = world().await;
    let (_late, early, _mid) = (
        slot_in(&w, 9).await,
        slot_in(&w, 3).await,
        slot_in(&w, 6).await,
    );
    // Not the earliest: a slot in the past, a busy one, and one of another service.
    let past = SlotRepository::create(
        &w.s.db,
        &Slot::new(
            w.schedule,
            Utc::now() - Duration::days(1),
            Utc::now() - Duration::days(1) + Duration::minutes(30),
        ),
    )
    .await
    .unwrap();
    assert!(past.start_datetime < Utc::now());
    let other = common::new_schedule_for(&w.s.db, &format!("other-{}", Uuid::new_v4()))
        .await
        .id;
    common::free_slot(&w.s.db, other, 1).await;

    let person = Uuid::new_v4();
    let (r, _) =
        w.s.referrals
            .create(
                new(&w, person, Some(&format!("U{}", Uuid::new_v4().simple()))),
                &sys(),
            )
            .await
            .unwrap();
    let b =
        w.s.referrals
            .book_first_available(r.id, &sys())
            .await
            .unwrap();
    assert_eq!(b.appointment.slot_id, Some(early));
    assert_eq!(b.appointment.person_id, person);
    assert_eq!(b.referral.status, "booked");
    assert_eq!(
        b.referral.appointment_id,
        Some(b.appointment.id),
        "the link is stored"
    );
    let stored =
        w.s.db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT referral_id FROM appointments WHERE id = $1",
                [b.appointment.id.into()],
            ))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        stored.try_get::<Option<Uuid>>("", "referral_id").unwrap(),
        Some(r.id)
    );
    assert_eq!(
        common::audit_count(&w.s.db, r.id, "booked_first_available").await,
        1
    );
    // Booked once.
    assert!(
        w.s.referrals
            .book_first_available(r.id, &sys())
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn with_nothing_free_the_referral_stays_open_and_says_so() {
    let w = world().await;
    let (r, _) =
        w.s.referrals
            .create(
                new(
                    &w,
                    Uuid::new_v4(),
                    Some(&format!("U{}", Uuid::new_v4().simple())),
                ),
                &sys(),
            )
            .await
            .unwrap();
    let err =
        w.s.referrals
            .book_first_available(r.id, &sys())
            .await
            .unwrap_err();
    assert!(
        err.to_string().contains("no free appointment") && err.to_string().contains("stays open"),
        "{err}"
    );
    assert_eq!(w.s.referrals.get(r.id).await.unwrap().status, "received");
    // A busy slot is not free either.
    let s = slot_in(&w, 4).await;
    w.s.scheduling
        .book_slot(s, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    assert!(
        w.s.referrals
            .book_first_available(r.id, &sys())
            .await
            .is_err()
    );
    // Once one is free, it books.
    slot_in(&w, 5).await;
    w.s.referrals
        .book_first_available(r.id, &sys())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_slot_that_clashes_with_the_patients_other_appointment_is_skipped() {
    let w = world().await;
    let person = Uuid::new_v4();
    let first = slot_in(&w, 3).await;
    let second = slot_in(&w, 6).await;
    // The patient already has something at the same time as the earliest slot, elsewhere.
    let other = common::new_schedule_for(&w.s.db, "elsewhere").await.id;
    let f = SlotRepository::find_by_id(&w.s.db, first)
        .await
        .unwrap()
        .unwrap();
    let clash =
        SlotRepository::create(&w.s.db, &Slot::new(other, f.start_datetime, f.end_datetime))
            .await
            .unwrap();
    w.s.scheduling
        .book_slot(clash.id, person, &sys())
        .await
        .unwrap();
    let (r, _) =
        w.s.referrals
            .create(
                new(&w, person, Some(&format!("U{}", Uuid::new_v4().simple()))),
                &sys(),
            )
            .await
            .unwrap();
    let b =
        w.s.referrals
            .book_first_available(r.id, &sys())
            .await
            .unwrap();
    assert_eq!(
        b.appointment.slot_id,
        Some(second),
        "the clashing slot was skipped"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_referral_can_be_waitlisted_and_closed_and_listed() {
    let w = world().await;
    let (r, _) =
        w.s.referrals
            .create(
                new(
                    &w,
                    Uuid::new_v4(),
                    Some(&format!("U{}", Uuid::new_v4().simple())),
                ),
                &sys(),
            )
            .await
            .unwrap();
    let wl =
        w.s.referrals
            .waitlist(r.id, Priority::Urgent, &sys())
            .await
            .unwrap();
    assert_eq!(wl.status, "waitlisted");
    let entries = w.s.waitlist.list_by_service(&w.service).await.unwrap();
    assert!(
        entries.iter().any(|e| e.referral_id == Some(r.id)),
        "the entry is linked to the referral"
    );
    assert!(
        w.s.referrals
            .waitlist(r.id, Priority::Routine, &sys())
            .await
            .is_err(),
        "once"
    );

    // A waitlisted referral can still be booked.
    slot_in(&w, 4).await;
    let b =
        w.s.referrals
            .book_first_available(r.id, &sys())
            .await
            .unwrap();
    assert_eq!(b.referral.status, "booked");

    let (c, _) =
        w.s.referrals
            .create(
                new(
                    &w,
                    Uuid::new_v4(),
                    Some(&format!("U{}", Uuid::new_v4().simple())),
                ),
                &sys(),
            )
            .await
            .unwrap();
    let closed =
        w.s.referrals
            .close(c.id, Some("sent elsewhere"), &sys())
            .await
            .unwrap();
    assert_eq!(closed.status, "closed");
    assert!(w.s.referrals.close(c.id, None, &sys()).await.is_err());
    assert!(
        w.s.referrals
            .book_first_available(c.id, &sys())
            .await
            .is_err(),
        "a closed referral is not booked"
    );

    let booked =
        w.s.referrals
            .list(Some("booked"), Some(&w.service), 100)
            .await
            .unwrap();
    assert!(booked.iter().any(|x| x.id == r.id) && booked.iter().all(|x| x.status == "booked"));
    assert!(
        w.s.referrals
            .list(Some("nonsense"), None, 10)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_receive_book_and_report() {
    let w = world().await;
    slot_in(&w, 4).await;
    let app = patient_administration_system::api::rest::router(w.s.clone());
    let call = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let mut b = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    b = b.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(b.body(body).unwrap()).await.unwrap();
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
    let person = Uuid::new_v4();
    let body = serde_json::json!({ "person_id": person, "source": "e-rs", "external_ref": format!("U{}", Uuid::new_v4().simple()), "target_service": w.service });
    let (st, v) = call("POST", "/api/referrals".into(), Some(body.clone())).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["duplicate"], false);
    let id = v["data"]["referral"]["id"].as_str().unwrap().to_string();
    let (_, v2) = call("POST", "/api/referrals".into(), Some(body)).await;
    assert_eq!(v2["data"]["duplicate"], true);
    let (st, _) = call(
        "POST",
        "/api/referrals".into(),
        Some(serde_json::json!({ "person_id": person, "source": "pigeon", "target_service": "x" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let (st, v) = call(
        "POST",
        format!("/api/referrals/{id}/book-first-available"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["referral"]["status"], "booked");
    let (st, v) = call(
        "POST",
        format!("/api/referrals/{id}/book-first-available"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    let (st, v) = call("GET", format!("/api/referrals/{id}"), None).await;
    assert_eq!(
        (st, v["data"]["status"].as_str()),
        (StatusCode::OK, Some("booked"))
    );
    let (st, _) = call("GET", format!("/api/referrals/{}", Uuid::new_v4()), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = call(
        "GET",
        format!("/api/referrals?status=booked&service={}", w.service),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"].as_array().unwrap().len(), 1);
}
