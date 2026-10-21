//! NOT-4 and CAS-9: delivery receipts, retry back-off and bounced contacts.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::reminders::ReminderService;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::{DeliveryChannel, LetterTemplate};
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

fn rid(tag: &str) -> String {
    format!("{tag}-{}", Uuid::new_v4())
}

fn sys() -> UserContext {
    UserContext::system()
}

struct Sms(Arc<Mutex<u32>>, bool);

#[async_trait]
impl MessageSender for Sms {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        _to: &str,
        _s: &str,
        _b: &str,
    ) -> patient_administration_system::Result<()> {
        *self.0.lock().unwrap() += 1;
        if self.1 {
            Ok(())
        } else {
            Err(patient_administration_system::Error::upstream("down"))
        }
    }
}

async fn world(works: bool) -> (AppState, Uuid, Uuid, Arc<Mutex<u32>>) {
    let attempts = Arc::new(Mutex::new(0));
    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    data.insert(
        "telecom".into(),
        serde_json::json!([{ "system": "phone", "value": "+15550001111" }]),
    );
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, Arc::new(Sms(attempts.clone(), works)));
    let mut t = LetterTemplate::new(
        format!("rc-{}", Uuid::new_v4()),
        "Hi".into(),
        "Hello.".into(),
    );
    t.channels = vec![DeliveryChannel::Sms];
    let template = LetterRepository::create_template(&s.db, &t)
        .await
        .unwrap()
        .id;
    (s, person, template, attempts)
}

async fn letter(s: &AppState, person: Uuid, template: Uuid) -> Uuid {
    s.communication
        .generate_letter(
            template,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap()
        .id
}

async fn status(s: &AppState, id: Uuid) -> String {
    s.db.query_one_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT status FROM generated_letters WHERE id = $1",
        [id.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get("", "status")
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_receipt_is_stored_once_however_often_the_carrier_repeats_it() {
    let (s, person, template, _) = world(true).await;
    let l = letter(&s, person, template).await;
    let same = rid("r-1");
    let r1 = s
        .receipts
        .record("acme", &same, l, "delivered", None, None, &sys())
        .await
        .unwrap();
    let r2 = s
        .receipts
        .record("acme", &same, l, "delivered", None, None, &sys())
        .await
        .unwrap();
    assert!(!r1.duplicate && r2.duplicate);
    assert_eq!(s.receipts.for_letter(l).await.unwrap().len(), 1);
    // The same id from another carrier is a different report.
    s.receipts
        .record("other", &same, l, "replied", None, None, &sys())
        .await
        .unwrap();
    assert_eq!(s.receipts.for_letter(l).await.unwrap().len(), 2);
    // Bad input.
    assert!(
        s.receipts
            .record("acme", &rid("r-2"), l, "exploded", None, None, &sys())
            .await
            .is_err()
    );
    assert!(
        s.receipts
            .record("", &rid("r-3"), l, "delivered", None, None, &sys())
            .await
            .is_err()
    );
    assert!(
        s.receipts
            .record(
                "acme",
                &rid("r-4"),
                Uuid::new_v4(),
                "delivered",
                None,
                None,
                &sys()
            )
            .await
            .is_err(),
        "unknown letter"
    );
    assert_eq!(
        status(&s, l).await,
        "sent",
        "delivered and replied leave a sent letter sent"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_bounce_marks_the_letter_failed_and_surfaces_the_contact_until_a_person_deals_with_it() {
    let (s, person, template, _) = world(true).await;
    let l = letter(&s, person, template).await;
    assert_eq!(status(&s, l).await, "sent");
    s.receipts
        .record(
            "acme",
            &rid("b-1"),
            l,
            "bounced",
            Some("invalid_number"),
            None,
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(
        status(&s, l).await,
        "failed",
        "a bounced message was not delivered"
    );

    let problems = s.receipts.contact_problems(90, 500).await.unwrap();
    let p = problems
        .iter()
        .find(|p| p.person_id == person)
        .expect("listed");
    assert_eq!(
        (p.bounces, p.last_channel.as_str(), p.last_reason.as_deref()),
        (1, "sms", Some("invalid_number"))
    );

    // A person fixes it: it leaves the list.
    assert!(
        s.receipts
            .resolve_problem(person, " ", None, &sys())
            .await
            .is_err()
    );
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    s.receipts
        .resolve_problem(person, "Clerk A", Some("rang, new number"), &sys())
        .await
        .unwrap();
    assert!(
        s.receipts
            .contact_problems(90, 500)
            .await
            .unwrap()
            .iter()
            .all(|p| p.person_id != person)
    );
    assert_eq!(
        common::audit_count(&s.db, person, "contact_problem_resolved").await,
        1
    );

    // A later bounce brings it back.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let l2 = letter(&s, person, template).await;
    s.receipts
        .record("acme", &rid("b-2"), l2, "bounced", None, None, &sys())
        .await
        .unwrap();
    let again = s.receipts.contact_problems(90, 500).await.unwrap();
    assert_eq!(
        again
            .iter()
            .find(|p| p.person_id == person)
            .unwrap()
            .bounces,
        1,
        "only bounces since it was dealt with"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_failed_reminder_is_retried_after_a_growing_delay_and_then_given_up_on() {
    let (s, person, _template, attempts) = world(false).await;
    let start = Utc::now() + Duration::hours(20);
    let mut a = Appointment::new(person, start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    let a = AppointmentRepository::create(&s.db, &a).await.unwrap();
    let svc = ReminderService::new(s.db.clone(), s.communication.clone());
    let lead = Duration::hours(24);
    let t0 = Utc::now();
    let tried = || *attempts.lock().unwrap();

    // Every failure happens at about t0 by the database's clock, so each scan is compared with t0 plus the
    // delay for the failures so far: 5 minutes after one, 15 after two, an hour, four hours, twelve.
    let r = svc.run_once(t0, lead).await.unwrap();
    assert_eq!(tried(), 1, "tried once: {r:?}");
    let r = svc
        .run_once(t0 + Duration::seconds(30), lead)
        .await
        .unwrap();
    assert!(r.backing_off >= 1, "straight away it backs off: {r:?}");
    assert_eq!(tried(), 1);
    for (offset, expect) in [
        (Duration::minutes(6), 2),
        (Duration::minutes(10), 2), // inside the fifteen
        (Duration::minutes(16), 3),
        (Duration::minutes(30), 3), // inside the hour
        (Duration::minutes(61), 4),
        (Duration::hours(2), 4), // inside the four hours
        (Duration::hours(4) + Duration::minutes(1), 5),
        (Duration::hours(8), 5), // inside the twelve
        (Duration::hours(12) + Duration::minutes(1), 6),
        (Duration::hours(13), 6), // the cap: no seventh
    ] {
        svc.run_once(t0 + offset, lead).await.unwrap();
        assert_eq!(tried(), expect, "at +{offset}");
    }
    let last = svc.run_once(t0 + Duration::hours(14), lead).await.unwrap();
    assert!(last.gave_up >= 1, "{last:?}");
    let template = svc.ensure_default_template().await.unwrap();
    assert_eq!(
        LetterRepository::count_failed_for_appointment(&s.db, a.id, template.id)
            .await
            .unwrap(),
        patient_administration_system::communication::reminders::MAX_FAILED_ATTEMPTS
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_take_receipts_list_problems_and_resolve_them() {
    let (s, person, template, _) = world(true).await;
    let l = letter(&s, person, template).await;
    let app = patient_administration_system::api::rest::router(s.clone());
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
    let body = serde_json::json!({ "carrier": "acme", "receipt_id": rid("h-1"), "letter_id": l, "event": "bounced", "detail": "mailbox_full" });
    let (st, v) = call(
        "POST",
        "/api/write-back/receipts".into(),
        Some(body.clone()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["duplicate"], false);
    let (_, v) = call("POST", "/api/write-back/receipts".into(), Some(body)).await;
    assert_eq!(v["data"]["duplicate"], true);
    let (st, _) = call("POST", "/api/write-back/receipts".into(), Some(serde_json::json!({ "carrier": "acme", "receipt_id": rid("h-2"), "letter_id": Uuid::new_v4(), "event": "delivered" }))).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = call("GET", format!("/api/letters/{l}/receipts"), None).await;
    assert_eq!(
        (st, v["data"][0]["event"].as_str()),
        (StatusCode::OK, Some("bounced"))
    );
    let (st, v) = call("GET", "/api/contact-problems?since_days=30".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["person_id"] == person.to_string())
    );
    let (st, _) = call(
        "POST",
        format!("/api/contact-problems/{person}/resolve"),
        Some(serde_json::json!({ "resolved_by": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        "POST",
        format!("/api/contact-problems/{person}/resolve"),
        Some(serde_json::json!({ "resolved_by": "Clerk A" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
}
