//! NEW-4: event subscriptions, delivered from the outbox with retries and signed webhooks. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use std::sync::{Arc, Mutex};

use axum::{Router, body::Bytes, extract::State, http::HeaderMap, http::StatusCode, routing::post};
use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::outbox::OutboxRepository;
use patient_administration_system::streaming::WebhookEventPublisher;
use patient_administration_system::subscriptions::SubscriptionService;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

/// What a receiver saw: the body and the signature of each request.
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

#[derive(Clone)]
struct Receiver {
    seen: Seen,
    /// Answers 500 for this many requests, then 200.
    fail_first: Arc<Mutex<u32>>,
}

async fn hook(State(r): State<Receiver>, headers: HeaderMap, body: Bytes) -> StatusCode {
    r.seen.lock().unwrap().push((
        String::from_utf8_lossy(&body).to_string(),
        headers
            .get("x-pas-signature")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
    ));
    let mut left = r.fail_first.lock().unwrap();
    if *left > 0 {
        *left -= 1;
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    }
}

/// A receiver on a free local port; returns its URL and what it has seen.
async fn receiver(fail_first: u32) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route("/hook", post(hook))
        .with_state(Receiver {
            seen: seen.clone(),
            fail_first: Arc::new(Mutex::new(fail_first)),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://127.0.0.1:{port}/hook"), seen)
}

/// Switch off every earlier subscription (earlier runs on a shared database left some with dead endpoints), so
/// a pass only has this test's own to work on.
async fn clean(db: &sea_orm::DatabaseConnection) {
    db.execute_unprepared(
        "UPDATE subscriptions SET status = 'off', ended_at = now() WHERE status = 'active'",
    )
    .await
    .unwrap();
}

async fn event(db: &sea_orm::DatabaseConnection, kind: &str, appointment: Uuid) {
    OutboxRepository::publish(
        db,
        kind,
        &serde_json::json!({"appointment_id": appointment}),
    )
    .await
    .unwrap();
}

async fn status_of(db: &sea_orm::DatabaseConnection, sub: Uuid) -> (i64, i64, i64) {
    let r = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) FILTER (WHERE status = 'pending') AS p, \
                    count(*) FILTER (WHERE status = 'delivered') AS d, \
                    count(*) FILTER (WHERE status = 'failed') AS f \
             FROM subscription_deliveries WHERE subscription_id = $1",
            [sub.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    (
        r.try_get("", "p").unwrap(),
        r.try_get("", "d").unwrap(),
        r.try_get("", "f").unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_change_reaches_each_subscriber_once_and_only_the_right_topic() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    clean(&state.db).await;
    let svc = SubscriptionService::new(state.db.clone());
    let ctx = UserContext::system();
    let (url_a, seen_a) = receiver(0).await;
    let (url_b, seen_b) = receiver(0).await;
    let (url_w, seen_w) = receiver(0).await;
    // SAFETY: tests in this binary run one at a time (--test-threads=1) and nothing else reads this name.
    unsafe { std::env::set_var("PAS_SUB_SECRET_NEW4A", "s3cret") };

    let a = svc
        .create(
            "appointment",
            &url_a,
            Some("PAS_SUB_SECRET_NEW4A"),
            "Admin Okafor",
            &ctx,
        )
        .await
        .unwrap();
    let b = svc
        .create("appointment", &url_b, None, "Admin Okafor", &ctx)
        .await
        .unwrap();
    let w = svc
        .create("waiting-list", &url_w, None, "Admin Okafor", &ctx)
        .await
        .unwrap();

    let appt = Uuid::new_v4();
    event(&state.db, "AppointmentBooked", appt).await;
    let now = Utc::now() + Duration::seconds(5);
    // Three passes: still once each.
    for _ in 0..3 {
        svc.tick(now).await.unwrap();
    }
    assert_eq!(seen_a.lock().unwrap().len(), 1);
    assert_eq!(seen_b.lock().unwrap().len(), 1);
    assert_eq!(
        seen_w.lock().unwrap().len(),
        0,
        "a waiting-list subscriber hears no appointment"
    );
    assert_eq!(status_of(&state.db, a.id).await, (0, 1, 0));
    assert_eq!(status_of(&state.db, w.id).await, (0, 0, 0));

    // The body is an R5 notification pointing at the appointment, signed with the named secret.
    let (body, sig) = seen_a.lock().unwrap()[0].clone();
    let n: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(n["type"], "subscription-notification");
    assert_eq!(
        n["entry"][0]["resource"]["notificationEvent"][0]["focus"]["reference"],
        format!("Appointment/{appt}")
    );
    assert_eq!(
        sig,
        Some(WebhookEventPublisher::sign(b"s3cret", body.as_bytes()))
    );
    assert_eq!(
        seen_b.lock().unwrap()[0].1,
        None,
        "no secret named, none sent"
    );

    // A subscription ended is not sent to.
    svc.end(b.id, &ctx).await.unwrap();
    event(&state.db, "AppointmentCancelled", Uuid::new_v4()).await;
    svc.tick(now).await.unwrap();
    assert_eq!(seen_a.lock().unwrap().len(), 2);
    assert_eq!(seen_b.lock().unwrap().len(), 1);
    assert!(
        svc.end(b.id, &ctx).await.is_err(),
        "ending twice is refused"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_failure_is_retried_later_survives_a_restart_and_is_given_up_on() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    clean(&state.db).await;
    let ctx = UserContext::system();
    let (url, seen) = receiver(2).await;
    let sub = SubscriptionService::new(state.db.clone())
        .create("appointment", &url, None, "Admin Okafor", &ctx)
        .await
        .unwrap();
    event(&state.db, "AppointmentBooked", Uuid::new_v4()).await;

    let t0 = Utc::now() + Duration::seconds(5);
    // First attempt fails: still pending, and not tried again before its delay.
    SubscriptionService::new(state.db.clone())
        .tick(t0)
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(status_of(&state.db, sub.id).await, (1, 0, 0));
    SubscriptionService::new(state.db.clone())
        .tick(t0 + Duration::seconds(10))
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1, "30 seconds have not passed");

    // A new service (a restart) picks the delivery up after the delay; the second try fails too.
    SubscriptionService::new(state.db.clone())
        .tick(t0 + Duration::seconds(31))
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    // The third succeeds, after the doubled delay.
    SubscriptionService::new(state.db.clone())
        .tick(t0 + Duration::seconds(31 + 61))
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(status_of(&state.db, sub.id).await, (0, 1, 0));
    let attempts: i32 = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT attempts FROM subscription_deliveries WHERE subscription_id = $1",
            [sub.id.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "attempts")
        .unwrap();
    assert_eq!(attempts, 3);

    // An endpoint that never answers well is given up on after the budget.
    let (dead, _) = receiver(1000).await;
    let doomed = SubscriptionService::new(state.db.clone())
        .create("appointment", &dead, None, "Admin Okafor", &ctx)
        .await
        .unwrap();
    event(&state.db, "AppointmentNoShow", Uuid::new_v4()).await;
    let mut t = Utc::now() + Duration::seconds(5);
    for _ in 0..12 {
        SubscriptionService::new(state.db.clone())
            .tick(t)
            .await
            .unwrap();
        t += Duration::hours(2);
    }
    assert_eq!(status_of(&state.db, doomed.id).await, (0, 0, 1));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn registration_refuses_what_it_should() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    clean(&state.db).await;
    let svc = SubscriptionService::new(state.db.clone());
    let ctx = UserContext::system();
    for (topic, endpoint, secret, by) in [
        ("referral", "https://p.example/h", None, "Admin"),
        ("appointment", "http://partner.example/h", None, "Admin"),
        (
            "appointment",
            "https://p.example/h",
            Some("API_TOKEN"),
            "Admin",
        ),
        ("appointment", "https://p.example/h", None, "  "),
    ] {
        assert!(
            svc.create(topic, endpoint, secret, by, &ctx).await.is_err(),
            "{topic} {endpoint} {secret:?} {by:?}"
        );
    }
    // An unset signing secret means nothing is sent, not that it is sent unsigned.
    let (url, seen) = receiver(0).await;
    let s = svc
        .create(
            "appointment",
            &url,
            Some("PAS_SUB_SECRET_NEVER_SET"),
            "Admin",
            &ctx,
        )
        .await
        .unwrap();
    event(&state.db, "AppointmentBooked", Uuid::new_v4()).await;
    svc.tick(Utc::now() + Duration::seconds(5)).await.unwrap();
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(status_of(&state.db, s.id).await, (1, 0, 0));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_subscriber_with_a_long_backlog_does_not_keep_another_waiting() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    clean(&state.db).await;
    let svc = SubscriptionService::new(state.db.clone());
    let ctx = UserContext::system();
    let (dead, _) = receiver(100_000).await;
    let (good, seen) = receiver(0).await;
    svc.create("appointment", &dead, None, "Admin", &ctx)
        .await
        .unwrap();
    svc.create("appointment", &good, None, "Admin", &ctx)
        .await
        .unwrap();
    for _ in 0..100 {
        event(&state.db, "AppointmentBooked", Uuid::new_v4()).await;
    }
    let now = Utc::now() + Duration::seconds(5);
    for _ in 0..8 {
        svc.tick(now).await.unwrap();
    }
    assert_eq!(
        seen.lock().unwrap().len(),
        100,
        "all reached the good subscriber"
    );
}
