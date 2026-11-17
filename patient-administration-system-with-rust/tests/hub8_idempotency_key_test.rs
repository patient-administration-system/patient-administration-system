//! HUB-8: a POST with an `Idempotency-Key` is done at most once per caller and key. Needs PostgreSQL:
//! `cargo test -- --ignored`.
mod common;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::sql::{Conn, Stmt};
use tower::ServiceExt;
use uuid::Uuid;

async fn post(
    s: &AppState,
    path: &str,
    caller: &str,
    key: Option<&str>,
    body: &str,
) -> (StatusCode, bool, String) {
    let mut req = Request::post(path)
        .header("content-type", "application/json")
        .header("x-user-id", caller);
    if let Some(k) = key {
        req = req.header("idempotency-key", k);
    }
    let resp = patient_administration_system::api::rest::router(s.clone())
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let replayed = resp.headers().get("idempotent-replayed").is_some();
    let text =
        String::from_utf8(to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
    (status, replayed, text)
}

async fn bookings(s: &AppState, slot: Uuid) -> u64 {
    s.db.count(Stmt::new(
        "SELECT count(*) AS n FROM appointments WHERE slot_id = $1",
        [slot.into()],
    ))
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_retried_booking_with_the_same_key_books_once_and_gets_the_same_answer() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await;
    let slot = common::free_slot(&s.db, schedule.id, 400).await;
    let path = format!("/api/slots/{}/book", slot.id);
    let body = format!(r#"{{"person_id":"{}"}}"#, Uuid::new_v4());
    let key = Uuid::new_v4().to_string();
    let first = post(&s, &path, "hub8-a", Some(&key), &body).await;
    assert!(first.0.is_success(), "{first:?}");
    assert!(!first.1);
    let again = post(&s, &path, "hub8-a", Some(&key), &body).await;
    assert_eq!((again.0, again.1), (first.0, true), "{again:?}");
    assert_eq!(again.2, first.2, "the same answer, byte for byte");
    assert_eq!(bookings(&s, slot.id).await, 1);

    // The same key for a different request is refused, not replayed.
    let other = format!(r#"{{"person_id":"{}"}}"#, Uuid::new_v4());
    let mismatch = post(&s, &path, "hub8-a", Some(&key), &other).await;
    assert_eq!(mismatch.0, StatusCode::UNPROCESSABLE_ENTITY, "{mismatch:?}");
    assert!(!mismatch.1);

    // Another caller's key is theirs: their request runs, and finds the slot taken.
    let theirs = post(&s, &path, "hub8-b", Some(&key), &body).await;
    assert!(!theirs.1);
    assert!(theirs.0.is_client_error(), "{theirs:?}");
    assert_eq!(bookings(&s, slot.id).await, 1);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn without_a_key_nothing_changes_and_a_bad_key_is_refused() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await;
    let slot = common::free_slot(&s.db, schedule.id, 401).await;
    let path = format!("/api/slots/{}/book", slot.id);
    let body = format!(r#"{{"person_id":"{}"}}"#, Uuid::new_v4());
    let bad = post(&s, &path, "hub8-a", Some("has a space"), &body).await;
    assert_eq!(bad.0, StatusCode::BAD_REQUEST, "{bad:?}");
    assert_eq!(bookings(&s, slot.id).await, 0, "a bad key runs nothing");
    let plain = post(&s, &path, "hub8-a", None, &body).await;
    assert!(plain.0.is_success(), "{plain:?}");
    let plain_again = post(&s, &path, "hub8-a", None, &body).await;
    assert!(!plain_again.1);
    assert!(
        plain_again.0.is_client_error(),
        "no key: the second runs and finds the slot taken"
    );
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_key_still_running_is_a_409_and_an_old_one_is_forgotten() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await;
    let slot = common::free_slot(&s.db, schedule.id, 402).await;
    let path = format!("/api/slots/{}/book", slot.id);
    let body = format!(r#"{{"person_id":"{}"}}"#, Uuid::new_v4());
    let key = Uuid::new_v4().to_string();
    let sha = |b: &[u8]| -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect()
    };
    let key_hash = sha(key.as_bytes());
    // The same request, as the layer fingerprints it: method, path and body.
    let request_hash = sha(format!("POST {path}\n{body}").as_bytes());
    s.db.run(Stmt::new(
        "INSERT INTO idempotency_keys (caller, key_sha256, request_sha256, state) VALUES ('hub8-c', $1, $2, 'processing')",
        [key_hash.clone().into(), request_hash.into()],
    ))
    .await
    .unwrap();
    // The same request while the first copy is still running: 409, and nothing runs.
    let running = post(&s, &path, "hub8-c", Some(&key), &body).await;
    assert_eq!(running.0, StatusCode::CONFLICT, "{running:?}");
    assert_eq!(bookings(&s, slot.id).await, 0);
    // Aged past 24 hours, the key is forgotten and the request runs.
    s.db.run(Stmt::new(
        "UPDATE idempotency_keys SET created_at = now() - interval '25 hours' WHERE caller = 'hub8-c' AND key_sha256 = $1",
        [key_hash.into()],
    ))
    .await
    .unwrap();
    let fresh = post(&s, &path, "hub8-c", Some(&key), &body).await;
    assert!(fresh.0.is_success(), "{fresh:?}");
    assert_eq!(bookings(&s, slot.id).await, 1);
}
