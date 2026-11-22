//! HUB-4, NFR-32: outbox delivery by failure class, with backoff and per-record order. Needs PostgreSQL:
//! `cargo test -- --ignored`.
//!
//! The events go to a mock subscriber (HUB-7) that answers by a word in the payload: `reject` → 400, `fail`
//! → 503, `slow` → 429 with `Retry-After: 120`, anything else → 204. The test events are dated in 2000, so they
//! are the oldest in a shared database and `tick_with_limit` settles only them.
mod common;
use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
use patient_administration_system::db::repositories::outbox::DeadLetterRepository;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Pool, Stmt};
use patient_administration_system::streaming::webhook::WebhookEventPublisher;
use patient_administration_system::streaming::{EventPublisher, dispatcher};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

type Hits = Arc<Mutex<HashMap<Uuid, u32>>>;

/// The mock subscriber: counts each event id it is sent and answers by the payload's `word`.
async fn subscriber() -> (String, Hits) {
    let hits: Hits = Arc::default();
    let h = hits.clone();
    let app = Router::new().route(
        "/hook",
        post(move |Json(ev): Json<serde_json::Value>| {
            let h = h.clone();
            async move {
                let id: Uuid = ev["id"].as_str().unwrap().parse().unwrap();
                *h.lock().unwrap().entry(id).or_default() += 1;
                match ev["payload"]["word"].as_str().unwrap_or("") {
                    "reject" => StatusCode::BAD_REQUEST.into_response(),
                    "fail" => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                    "slow" => {
                        (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "120")]).into_response()
                    }
                    _ => StatusCode::NO_CONTENT.into_response(),
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/hook"), hits)
}

struct World {
    db: Pool,
    publisher: Arc<dyn EventPublisher>,
    hits: Hits,
    ids: Vec<Uuid>,
}

impl World {
    async fn new() -> Self {
        let db = common::build_state(&common::database_url().unwrap())
            .await
            .db;
        let (url, hits) = subscriber().await;
        let publisher: Arc<dyn EventPublisher> =
            Arc::new(WebhookEventPublisher::new(&url, None, Duration::from_secs(5)).unwrap());
        // Anything left from an earlier run of these tests (dated 2000) would take the first places.
        db.run(Stmt::text(
            "DELETE FROM outbox_events WHERE at < '2001-01-01' AND payload ? 'hub4'",
        ))
        .await
        .unwrap();
        db.run(Stmt::text(
            "DELETE FROM outbox_dead_letters WHERE created_at < '2001-01-01' AND payload ? 'hub4'",
        ))
        .await
        .unwrap();
        Self {
            db,
            publisher,
            hits,
            ids: Vec::new(),
        }
    }

    /// An event for `person`, carrying `word`, `n` seconds into 2000.
    async fn event(&mut self, person: Uuid, word: &str, n: i64) -> Uuid {
        let id = Uuid::new_v4();
        self.db
            .run(Stmt::new(
                "INSERT INTO outbox_events (id, event_type, payload, published, at, retry_count) \
                 VALUES ($1, 'Hub4Test', jsonb_build_object('person_id', $2::text, 'word', $3::text, 'hub4', true), false, \
                 timestamptz '2000-01-01 00:00:00+00' + make_interval(secs => $4::int), 0)",
                [id.into(), person.to_string().into(), word.into(), n.into()],
            ))
            .await
            .unwrap();
        self.ids.push(id);
        id
    }

    async fn tick(&self) {
        dispatcher::tick_with_limit(&self.db, &self.publisher, 10, self.ids.len() as u64)
            .await
            .unwrap();
    }

    fn hits(&self, id: Uuid) -> u32 {
        self.hits.lock().unwrap().get(&id).copied().unwrap_or(0)
    }

    /// `(published, retry_count, seconds until next_attempt_at)`, or `None` when the event is not in the outbox.
    async fn row(&self, id: Uuid) -> Option<(bool, i32, Option<f64>)> {
        self.db
            .one(Stmt::new(
                "SELECT published, retry_count, extract(epoch FROM next_attempt_at - now())::float8 AS wait \
                 FROM outbox_events WHERE id = $1",
                [id.into()],
            ))
            .await
            .unwrap()
            .map(|r| {
                (
                    r.col("published").unwrap(),
                    r.col("retry_count").unwrap(),
                    r.col("wait").unwrap(),
                )
            })
    }

    async fn dead_letter(&self, id: Uuid) -> Option<(Uuid, String)> {
        self.db
            .one(Stmt::new(
                "SELECT id, last_error FROM outbox_dead_letters WHERE original_id = $1",
                [id.into()],
            ))
            .await
            .unwrap()
            .map(|r| (r.col("id").unwrap(), r.col("last_error").unwrap()))
    }

    async fn clean(&self) {
        for id in &self.ids {
            for sql in [
                "DELETE FROM outbox_events WHERE id = $1",
                "DELETE FROM outbox_dead_letters WHERE original_id = $1",
            ] {
                self.db.run(Stmt::new(sql, [(*id).into()])).await.unwrap();
            }
        }
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_rejected_event_is_dead_lettered_at_once_and_a_good_one_is_delivered() {
    let mut w = World::new().await;
    let bad = w.event(Uuid::new_v4(), "reject", 1).await;
    let good = w.event(Uuid::new_v4(), "ok", 2).await;
    w.tick().await;
    assert_eq!((w.hits(bad), w.hits(good)), (1, 1));
    assert_eq!(w.row(bad).await, None, "moved out of the outbox");
    let (_, why) = w
        .dead_letter(bad)
        .await
        .expect("dead-lettered after one try, not ten");
    assert!(why.starts_with("rejected: "), "{why}");
    assert!(why.contains("400"), "{why}");
    assert_eq!(w.row(good).await.map(|r| r.0), Some(true));
    w.clean().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_deferred_event_backs_off_instead_of_being_tried_every_tick() {
    let mut w = World::new().await;
    let flaky = w.event(Uuid::new_v4(), "fail", 1).await;
    w.tick().await;
    let (published, count, wait) = w.row(flaky).await.unwrap();
    assert!(!published);
    assert_eq!(count, 1);
    let wait = wait.expect("a next attempt time");
    assert!(
        (3.0..=6.0).contains(&wait),
        "the first wait is about 5 s: {wait}"
    );
    // The next tick, at once, does not try it again.
    w.tick().await;
    assert_eq!(w.hits(flaky), 1);
    assert_eq!(w.dead_letter(flaky).await, None);
    w.clean().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn retry_after_is_honoured() {
    let mut w = World::new().await;
    let slow = w.event(Uuid::new_v4(), "slow", 1).await;
    w.tick().await;
    let wait = w.row(slow).await.unwrap().2.unwrap();
    assert!((115.0..=121.0).contains(&wait), "Retry-After: 120 → {wait}");
    w.clean().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_later_event_for_the_same_person_waits_for_the_earlier_one() {
    let mut w = World::new().await;
    let person = Uuid::new_v4();
    let first = w.event(person, "fail", 1).await;
    let second = w.event(person, "ok", 2).await;
    let other = w.event(Uuid::new_v4(), "ok", 3).await;
    w.tick().await;
    assert_eq!(w.hits(first), 1);
    assert_eq!(w.hits(second), 0, "it must not overtake the failing one");
    assert_eq!(w.hits(other), 1, "another person's event is not held up");
    // Once the first is delivered, the second follows.
    w.db.run(Stmt::new(
        "UPDATE outbox_events SET next_attempt_at = NULL, payload = payload || '{\"word\": \"ok\"}' WHERE id = $1",
        [first.into()],
    ))
    .await
    .unwrap();
    w.tick().await;
    assert_eq!(w.row(first).await.map(|r| r.0), Some(true));
    w.tick().await;
    assert_eq!(w.row(second).await.map(|r| r.0), Some(true));
    w.clean().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_dead_letter_holds_its_person_until_a_person_skips_it() {
    let mut w = World::new().await;
    let person = Uuid::new_v4();
    let first = w.event(person, "reject", 1).await;
    let second = w.event(person, "ok", 2).await;
    w.tick().await;
    let (dl, _) = w.dead_letter(first).await.unwrap();
    w.tick().await;
    assert_eq!(w.hits(second), 0, "held behind the dead letter");
    let held =
        patient_administration_system::db::repositories::outbox::OutboxRepository::count_held(
            &w.db,
        )
        .await
        .unwrap();
    assert!(held >= 1);
    // Skipping releases it; the dead letter is gone and the next event is delivered.
    let (event_type, original) = DeadLetterRepository::skip(&w.db, dl).await.unwrap();
    assert_eq!((event_type.as_str(), original), ("Hub4Test", first));
    w.tick().await;
    assert_eq!(w.hits(second), 1);
    assert_eq!(w.row(second).await.map(|r| r.0), Some(true));
    w.clean().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn an_event_that_keeps_failing_is_dead_lettered_when_the_retry_budget_is_spent() {
    let mut w = World::new().await;
    let flaky = w.event(Uuid::new_v4(), "fail", 1).await;
    // A budget of one: the first failure is the last try.
    dispatcher::tick_with_limit(&w.db, &w.publisher, 1, 1)
        .await
        .unwrap();
    assert_eq!(w.hits(flaky), 1);
    assert_eq!(w.row(flaky).await, None, "moved out of the outbox");
    let (_, why) = w.dead_letter(flaky).await.expect("dead-lettered");
    assert!(
        !why.starts_with("rejected: "),
        "a deferral, not a rejection: {why}"
    );
    w.clean().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_dispatcher_loop_delivers_what_is_due_while_it_runs() {
    let mut w = World::new().await;
    let good = w.event(Uuid::new_v4(), "ok", 1).await;
    let handle = tokio::spawn(dispatcher::run(
        w.db.clone(),
        w.publisher.clone(),
        Duration::from_millis(50),
        10,
    ));
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if w.row(good).await.map(|r| r.0) == Some(true) {
            break;
        }
    }
    handle.abort();
    assert_eq!(
        w.row(good).await.map(|r| r.0),
        Some(true),
        "delivered by the loop"
    );
    assert_eq!(w.hits(good), 1);
    w.clean().await;
}
