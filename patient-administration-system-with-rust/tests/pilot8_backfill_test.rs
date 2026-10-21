//! PILOT-8: the backfill loop. Needs PostgreSQL: `cargo test -- --ignored --test-threads=1`
//! (one test switches `PAS_BACKFILL_OFFERS` on for itself, so run these single-threaded).

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::backfill::BackfillService;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{Appointment, CancellationReason};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::standby::StandbyInput;
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct Capture(Arc<Mutex<Vec<(String, String)>>>);

#[async_trait]
impl MessageSender for Capture {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _s: &str,
        body: &str,
    ) -> patient_administration_system::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((to.to_string(), body.to_string()));
        Ok(())
    }
}

struct W {
    s: AppState,
    svc: BackfillService,
    service: String,
    schedule: Uuid,
    sent: Arc<Mutex<Vec<(String, String)>>>,
    people: Vec<(Uuid, String)>,
}

async fn world(n_people: usize) -> W {
    // These tests are about who is offered what, not when texts may be sent; without this they fail between
    // 21:00 and 08:00 clinic time, when the default quiet hours hold every text back.
    // SAFETY: tests that set environment variables run one at a time (`--test-threads=1`).
    unsafe { std::env::set_var("PAS_QUIET_HOURS", "off") };
    let sent = Arc::new(Mutex::new(Vec::new()));
    let stub = Arc::new(StubPersonClient::new());
    let mut people = Vec::new();
    for _ in 0..n_people {
        let id = Uuid::new_v4();
        let phone =
            format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000).replace(' ', "");
        let mut data = serde_json::Map::new();
        data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
        data.insert(
            "telecom".into(),
            serde_json::json!([{ "system": "phone", "value": phone }]),
        );
        stub.insert(Person { id, data });
        people.push((id, phone));
    }
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture(sent.clone())));
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    let svc = BackfillService::new(s.db.clone(), s.standby.clone(), s.communication.clone())
        .with_link_base(Some("https://pas.test".into()));
    W {
        s,
        svc,
        service,
        schedule,
        sent,
        people,
    }
}

impl W {
    async fn join_standby(&self) {
        for (p, _) in &self.people {
            self.s
                .standby
                .add(
                    StandbyInput {
                        person_id: *p,
                        service_type: self.service.clone(),
                        short_notice_ok: true,
                        language: None,
                        needs_transport: false,
                        phone_only: false,
                    },
                    &sys(),
                )
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    /// A patient books a slot five days ahead and gives it up; returns the release id and slot.
    async fn release(&self) -> (Uuid, Uuid, Appointment) {
        let slot = common::free_slot(&self.s.db, self.schedule, 5).await;
        let a = self
            .s
            .scheduling
            .book_slot(slot.id, Uuid::new_v4(), &sys())
            .await
            .unwrap();
        self.s
            .scheduling
            .cancel(a.id, CancellationReason::PatientRequest, &sys())
            .await
            .unwrap();
        let r = self
            .s
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id FROM slot_releases WHERE slot_id = $1 AND status = 'open'",
                [slot.id.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        (r.try_get("", "id").unwrap(), slot.id, a)
    }
    async fn statuses(&self, release: Uuid) -> Vec<String> {
        self.svc
            .offers_for_release(release)
            .await
            .unwrap()
            .iter()
            .map(|o| o["status"].as_str().unwrap().to_string())
            .collect()
    }
    async fn expire_live_offer(&self, release: Uuid) {
        self.s
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE slot_offers SET expires_at = now() - interval '1 minute' WHERE release_id = $1 AND status = 'open'",
                [release.into()],
            ))
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn offers_go_in_standby_order_one_at_a_time_and_move_on_when_declined_or_expired() {
    let w = world(3).await;
    w.join_standby().await;
    let (release, _slot, _) = w.release().await;
    let (p1, p2, p3) = (w.people[0].0, w.people[1].0, w.people[2].0);

    let (o1, t1) = w
        .svc
        .offer_next(release)
        .await
        .unwrap()
        .expect("first offer");
    assert_eq!(o1.person_id, p1, "longest on standby first");
    assert!(
        w.svc.offer_next(release).await.unwrap().is_none(),
        "one live offer at a time"
    );

    // p1 declines: p2 is offered at once.
    w.svc.decline(&t1).await.unwrap();
    assert_eq!(w.statuses(release).await, ["declined", "open"]);

    // p2's offer runs out: the scan expires it and offers p3.
    w.expire_live_offer(release).await;
    let rep = w.svc.tick(Utc::now()).await.unwrap();
    assert!(rep.expired >= 1 && rep.offered >= 1, "{rep:?}");
    assert_eq!(w.statuses(release).await, ["declined", "expired", "open"]);

    // The live offer is p3's, the only one never offered before.
    let live =
        w.s.db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT person_id FROM slot_offers WHERE release_id = $1 AND status = 'open'",
                [release.into()],
            ))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(live.try_get::<Uuid>("", "person_id").unwrap(), p3);
    assert_ne!(p2, p3);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn accepting_books_the_slot_for_that_patient_takes_them_off_standby_and_stops_the_loop() {
    let w = world(2).await;
    w.join_standby().await;
    let (release, slot, _) = w.release().await;
    let (o, token) = w.svc.offer_next(release).await.unwrap().unwrap();
    let out = w.svc.accept(&token, &sys()).await.unwrap();
    assert_eq!(out.status, "accepted");
    // Booked for the patient who accepted, on that slot, and the release is refilled.
    let appt = w
        .s
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT person_id, status FROM appointments WHERE slot_id = $1 AND status = 'booked'",
            [slot.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(appt.try_get::<Uuid>("", "person_id").unwrap(), o.person_id);
    let rel =
        w.s.db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT status FROM slot_releases WHERE id = $1",
                [release.into()],
            ))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(rel.try_get::<String>("", "status").unwrap(), "refilled");
    let still_on = w.s.standby.list(Some(&w.service)).await.unwrap();
    assert!(
        still_on.iter().all(|e| e.person_id != o.person_id),
        "off standby"
    );
    // Nothing more is offered, and the link works once.
    assert!(w.svc.offer_next(release).await.unwrap().is_none());
    assert!(w.svc.accept(&token, &sys()).await.is_err());
    assert_eq!(w.statuses(release).await, ["accepted"]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn two_taps_at_once_act_once() {
    let w = world(1).await;
    w.join_standby().await;
    let (release, slot, _) = w.release().await;
    let (_, token) = w.svc.offer_next(release).await.unwrap().unwrap();
    let ctx = sys();
    let (a, b) = tokio::join!(w.svc.accept(&token, &ctx), w.svc.accept(&token, &ctx));
    assert_eq!([a.is_ok(), b.is_ok()].iter().filter(|x| **x).count(), 1);
    let n =
        w.s.db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM appointments WHERE slot_id = $1 AND status = 'booked'",
                [slot.into()],
            ))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(n.try_get::<i64>("", "n").unwrap(), 1, "exactly one booking");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_expired_offer_cannot_be_accepted_and_is_time_limited() {
    let w = world(1).await;
    w.join_standby().await;
    let (release, _slot, _) = w.release().await;
    let (o, token) = w.svc.offer_next(release).await.unwrap().unwrap();
    assert!(o.expires_at > Utc::now() && o.expires_at <= Utc::now() + Duration::minutes(31));
    assert_eq!(w.svc.info(&token).await.unwrap().state, "open");
    w.expire_live_offer(release).await;
    assert_eq!(w.svc.info(&token).await.unwrap().state, "expired");
    assert!(w.svc.accept(&token, &sys()).await.is_err());
    assert!(w.svc.decline(&token).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_slot_taken_meanwhile_is_not_double_booked_and_the_next_patient_is_tried() {
    let w = world(2).await;
    w.join_standby().await;
    let (release, slot, _) = w.release().await;
    let (_, token) = w.svc.offer_next(release).await.unwrap().unwrap();
    // Someone else books the slot directly.
    w.s.scheduling
        .book_slot(slot, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    let out = w.svc.accept(&token, &sys()).await.unwrap();
    assert_eq!(out.status, "gone");
    assert_eq!(
        w.statuses(release).await,
        ["missed"],
        "the slot is refilled, so there is no one to offer next"
    );
    let n =
        w.s.db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM appointments WHERE slot_id = $1 AND status = 'booked'",
                [slot.into()],
            ))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(n.try_get::<i64>("", "n").unwrap(), 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn opening_the_link_changes_nothing_and_a_bad_token_is_just_not_valid() {
    let w = world(1).await;
    w.join_standby().await;
    let (release, _, _) = w.release().await;
    let (_, token) = w.svc.offer_next(release).await.unwrap().unwrap();
    for _ in 0..5 {
        let i = w.svc.info(&token).await.unwrap();
        assert_eq!(i.state, "open");
        assert!(i.starts_at.is_some());
    }
    assert_eq!(w.statuses(release).await, ["open"]);
    assert!(w.svc.info(&"0".repeat(64)).await.is_err());
    assert!(w.svc.info("short").await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_offer_is_messaged_with_a_link_and_an_opted_out_patient_is_not() {
    let w = world(2).await;
    w.join_standby().await;
    let (release, _, _) = w.release().await;
    // The first patient has opted out of texts.
    patient_administration_system::communication::consent::record_optout(
        &w.s.db,
        &patient_administration_system::communication::consent::person_subject(w.people[0].0),
        "sms",
        "staff",
        None,
    )
    .await
    .unwrap();
    assert!(w.svc.offer_and_notify(release).await.unwrap());
    assert!(
        w.sent
            .lock()
            .unwrap()
            .iter()
            .all(|(to, _)| *to != w.people[0].1),
        "no text to an opted-out patient"
    );
    let sent_before = w.sent.lock().unwrap().len();
    assert_eq!(sent_before, 0);
    // Their offer runs out unanswered and the next patient, who can be texted, gets it.
    w.expire_live_offer(release).await;
    w.svc.tick(Utc::now()).await.unwrap();
    let sent = w.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, w.people[1].1);
    assert!(sent[0].1.contains("https://pas.test/o/"), "{}", sent[0].1);
    let token = sent[0]
        .1
        .split("https://pas.test/o/")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .trim_end_matches('.');
    assert_eq!(token.len(), 64, "an opaque token");
    assert!(!sent[0].1.contains(&w.people[1].0.to_string()));
    assert_eq!(w.svc.info(token).await.unwrap().state, "open");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_release_makes_an_offer_by_itself_only_when_switched_on() {
    let w = world(2).await;
    w.join_standby().await;
    let (release, _, _) = w.release().await;
    assert!(
        w.statuses(release).await.is_empty(),
        "off by default: nothing is offered"
    );

    // SAFETY: tests that read this variable run single-threaded (see the header).
    unsafe { std::env::set_var("PAS_BACKFILL_OFFERS", "true") };
    let (release2, _, _) = w.release().await;
    unsafe { std::env::remove_var("PAS_BACKFILL_OFFERS") };
    assert_eq!(
        w.statuses(release2).await,
        ["open"],
        "the cancel offered the slot to the first patient"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_read_without_acting_and_act_on_post() {
    let w = world(2).await;
    w.join_standby().await;
    let (release, _, _) = w.release().await;
    let (_, token) = w.svc.offer_next(release).await.unwrap().unwrap();
    let app = patient_administration_system::api::rest::router(w.s.clone());
    let call = |method: &'static str, uri: String| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
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
    let (st, v) = call("GET", format!("/api/offer-links/{token}")).await;
    assert_eq!(
        (st, v["data"]["state"].as_str()),
        (StatusCode::OK, Some("open"))
    );
    let (st, _) = call("GET", format!("/api/offer-links/{token}/accept")).await;
    assert_eq!(st, StatusCode::METHOD_NOT_ALLOWED, "a GET never acts");
    let (st, _) = call("POST", format!("/api/offer-links/{token}/frobnicate")).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call("GET", format!("/api/offer-links/{}", "0".repeat(64))).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = call("POST", format!("/api/offer-links/{token}/accept")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["status"], "accepted");
    let (st, _) = call("POST", format!("/api/offer-links/{token}/accept")).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, v) = call("GET", format!("/api/slot-releases/{release}/offers")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"][0]["status"], "accepted");
}

/// QUAL-25: quiet hours decided by the service's clock, not the wall clock. The same offer text is held at
/// 22:00 clinic time and sent at noon, whenever this test runs, with quiet hours on.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn quiet_hours_hold_an_offer_text_by_the_clinic_clock_whatever_the_time_of_the_run() {
    use chrono::TimeZone;
    use patient_administration_system::clock::{Clock, FixedClock};
    let w = world(2).await;
    // SAFETY: tests that set environment variables run one at a time (`--test-threads=1`).
    unsafe { std::env::set_var("PAS_QUIET_HOURS", "21:00-08:00") };
    w.join_standby().await;
    let (release, _slot, _) = w.release().await;
    let london = chrono_tz::Europe::London;
    let today = Utc::now().with_timezone(&london).date_naive();
    let at = |h: u32| {
        london
            .from_local_datetime(&today.and_hms_opt(h, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .with_timezone(&Utc)
    };
    let clock = FixedClock::at(at(22));
    w.svc.set_clock(clock.clone());

    w.svc
        .offer_next(release)
        .await
        .unwrap()
        .expect("an offer at 22:00");
    assert!(
        w.sent.lock().unwrap().is_empty(),
        "no text at 22:00 clinic time"
    );

    // The offer runs out unsent and the next patient is offered at noon: that one is texted.
    // Expired by the fixed clock's reckoning, not the database's `now()`, which is the wall clock.
    clock.set(at(12));
    w.s.db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE slot_offers SET expires_at = $2 WHERE release_id = $1 AND status = 'open'",
            [release.into(), (clock.now() - Duration::minutes(1)).into()],
        ))
        .await
        .unwrap();
    w.svc.tick(clock.now()).await.unwrap();
    let sent = w.sent.lock().unwrap().clone();
    unsafe { std::env::set_var("PAS_QUIET_HOURS", "off") };
    assert_eq!(sent.len(), 1, "one text at noon: {sent:?}");
    assert_eq!(sent[0].0, w.people[1].1, "to the second patient on standby");
}
