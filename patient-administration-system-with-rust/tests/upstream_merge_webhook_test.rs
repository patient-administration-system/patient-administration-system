//! Integration tests for the v0.44.0 Phase 3a inbound merge webhooks:
//! `POST /api/upstream/person-merged` and
//! `POST /api/upstream/worker-merged`.
//!
//! These tests do touch the DB — they exercise `upstream_inbox` dedupe
//! and audit_log writes — so they gate on `DATABASE_URL`. They run
//! against a freshly-migrated DB.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;
use tower::ServiceExt;
use uuid::Uuid;

/// The tests below set and clear process-wide environment variables (the webhook
/// secrets). Cargo runs a binary's tests on parallel threads, so they take this
/// lock to run one at a time.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret).unwrap();
    mac.update(body);
    format!(
        "sha256={}",
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_person_merged_webhook_without_secret_returns_503() {
    let _env_guard = ENV_LOCK.lock().await;
    let Some(url) = common::database_url() else {
        return;
    };
    // SAFETY: tests are serial via cargo's per-binary process model; only
    // one test in this binary touches PERSON_SERVICE_WEBHOOK_SECRET
    // at a time.
    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
    }
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);

    let body = serde_json::json!({
        "source_id": Uuid::new_v4(),
        "target_id": Uuid::new_v4(),
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_person_merged_webhook_wrong_signature_returns_401() {
    let _env_guard = ENV_LOCK.lock().await;
    let Some(url) = common::database_url() else {
        return;
    };
    unsafe {
        std::env::set_var("PERSON_SERVICE_WEBHOOK_SECRET", "right");
    }
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);

    let body = serde_json::json!({
        "source_id": Uuid::new_v4(),
        "target_id": Uuid::new_v4(),
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let bad_sig = sign(b"wrong", &body_bytes);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", bad_sig)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let env = body_json(resp).await;
    assert_eq!(env["code"], serde_json::json!("unsigned_webhook"));
    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_person_merged_webhook_happy_path_applied_then_dedupe() {
    let _env_guard = ENV_LOCK.lock().await;
    let Some(url) = common::database_url() else {
        return;
    };
    unsafe {
        std::env::set_var("PERSON_SERVICE_WEBHOOK_SECRET", "shared");
    }
    let state = common::build_state(&url).await;

    // Two fresh PAS-owned ids; nothing else in the DB references them.
    let body = serde_json::json!({
        "source_id": Uuid::new_v4(),
        "target_id": Uuid::new_v4(),
        "event_id": Uuid::new_v4(),
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let sig = sign(b"shared", &body_bytes);

    // First call — applied.
    let resp = patient_administration_system::api::rest::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", &sig)
                .body(Body::from(body_bytes.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["data"]["applied"], Value::Bool(true));

    // Second call with the same body — duplicate (idempotent).
    let resp = patient_administration_system::api::rest::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", &sig)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["data"]["applied"], Value::Bool(false));
    assert_eq!(env["data"]["reason"], serde_json::json!("duplicate"));

    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_person_merged_self_merge_returns_400() {
    let _env_guard = ENV_LOCK.lock().await;
    let Some(url) = common::database_url() else {
        return;
    };
    unsafe {
        std::env::set_var("PERSON_SERVICE_WEBHOOK_SECRET", "shared");
    }
    let state = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(state);
    let same = Uuid::new_v4();
    let body = serde_json::json!({ "source_id": same, "target_id": same });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let sig = sign(b"shared", &body_bytes);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/person-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", sig)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn test_worker_merged_webhook_happy_path() {
    let _env_guard = ENV_LOCK.lock().await;
    let Some(url) = common::database_url() else {
        return;
    };
    unsafe {
        std::env::set_var("WORKER_SERVICE_WEBHOOK_SECRET", "wsecret");
    }
    let state = common::build_state(&url).await;
    let body = serde_json::json!({
        "source_id": Uuid::new_v4(),
        "target_id": Uuid::new_v4(),
        "event_id": Uuid::new_v4(),
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let sig = sign(b"wsecret", &body_bytes);
    let resp = patient_administration_system::api::rest::router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upstream/worker-merged")
                .header("content-type", "application/json")
                .header("x-upstream-signature", sig)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let env = body_json(resp).await;
    assert_eq!(env["data"]["applied"], Value::Bool(true));
    unsafe {
        std::env::remove_var("WORKER_SERVICE_WEBHOOK_SECRET");
    }
}

async fn post_webhook(
    app: &axum::Router,
    path: &str,
    body: &[u8],
    signature: Option<&str>,
    event_header: Option<&str>,
) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(s) = signature {
        b = b.header("x-upstream-signature", s);
    }
    if let Some(e) = event_header {
        b = b.header("x-upstream-event-id", e);
    }
    let resp = app
        .clone()
        .oneshot(b.body(Body::from(body.to_vec())).unwrap())
        .await
        .unwrap();
    let st = resp.status();
    (st, body_json(resp).await)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_worker_webhook_refuses_what_it_cannot_trust_and_ignores_a_replay() {
    let _env_guard = ENV_LOCK.lock().await;
    let url = common::database_url().unwrap();
    let app = patient_administration_system::api::rest::router(common::build_state(&url).await);
    let path = "/api/upstream/worker-merged";
    let body = serde_json::to_vec(&serde_json::json!({
        "source_id": Uuid::new_v4(), "target_id": Uuid::new_v4()
    }))
    .unwrap();

    // No secret configured: refused outright.
    unsafe { std::env::remove_var("WORKER_SERVICE_WEBHOOK_SECRET") };
    let (st, _) = post_webhook(&app, path, &body, Some(&sign(b"s", &body)), None).await;
    assert!(st.is_server_error(), "{st}");

    unsafe { std::env::set_var("WORKER_SERVICE_WEBHOOK_SECRET", "wsecret") };
    // Every way a signature can be wrong is a 401.
    for bad in [
        None,
        Some("deadbeef"),
        Some("sha256=zz"),
        Some("sha256=abc"),
        Some("sha256=00"),
    ] {
        let (st, _) = post_webhook(&app, path, &body, bad, None).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "{bad:?}");
    }
    let wrong = sign(b"other", &body);
    let (st, _) = post_webhook(&app, path, &body, Some(&wrong), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // Signed, but not a merge: malformed, and a merge of a record into itself.
    let junk = b"not json".to_vec();
    let (st, _) = post_webhook(&app, path, &junk, Some(&sign(b"wsecret", &junk)), None).await;
    assert!(st.is_client_error());
    let same = Uuid::new_v4();
    let selfmerge =
        serde_json::to_vec(&serde_json::json!({"source_id": same, "target_id": same})).unwrap();
    let (st, _) = post_webhook(
        &app,
        path,
        &selfmerge,
        Some(&sign(b"wsecret", &selfmerge)),
        None,
    )
    .await;
    assert!(st.is_client_error());

    // Without an event id the pair makes one, so the same message twice is applied once; an id in the header
    // wins over the pair's, so the same pair under a new header is a new event.
    let good = sign(b"wsecret", &body);
    let (st, v) = post_webhook(&app, path, &body, Some(&good), None).await;
    assert_eq!(
        (st, &v["data"]["applied"]),
        (StatusCode::OK, &Value::Bool(true))
    );
    let (_, v) = post_webhook(&app, path, &body, Some(&good), None).await;
    assert_eq!(v["data"]["applied"], Value::Bool(false));
    assert_eq!(v["data"]["reason"], "duplicate");
    let header_id = Uuid::new_v4().to_string();
    let (_, v) = post_webhook(&app, path, &body, Some(&good), Some(&header_id)).await;
    assert_eq!(v["data"]["applied"], Value::Bool(true));
    let (_, v) = post_webhook(&app, path, &body, Some(&good), Some(&header_id)).await;
    assert_eq!(v["data"]["applied"], Value::Bool(false));
    let (_, v) = post_webhook(&app, path, &body, Some(&good), Some("not-a-uuid")).await;
    assert_eq!(
        v["data"]["applied"],
        Value::Bool(false),
        "an unreadable header falls back to the pair"
    );

    unsafe { std::env::remove_var("WORKER_SERVICE_WEBHOOK_SECRET") };
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_merge_moves_the_rows_of_the_old_record_to_the_new_one_and_audits_it() {
    use patient_administration_system::db::row::RowExt;
    use patient_administration_system::db::sql::{Conn, Stmt};
    let _env_guard = ENV_LOCK.lock().await;
    let url = common::database_url().unwrap();
    let state = common::build_state(&url).await;
    let db = state.db.clone();
    let app = patient_administration_system::api::rest::router(state);
    unsafe {
        std::env::set_var("PERSON_SERVICE_WEBHOOK_SECRET", "psecret");
        std::env::set_var("WORKER_SERVICE_WEBHOOK_SECRET", "wsecret");
    }
    let (old_person, new_person) = (Uuid::new_v4(), Uuid::new_v4());
    let (old_worker, new_worker) = (Uuid::new_v4(), Uuid::new_v4());
    let appt = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO appointments (id, person_id, worker_id, start_datetime, end_datetime, status, created_at, updated_at) \
         VALUES ($1,$2,$3, now() + interval '30 days', now() + interval '30 days 30 minutes', 'booked', now(), now())",
        [appt.into(), old_person.into(), old_worker.into()],
    ))
    .await
    .unwrap();

    let send = |path: &'static str, secret: &'static [u8], src: Uuid, dst: Uuid| {
        let app = app.clone();
        async move {
            let body = serde_json::to_vec(&serde_json::json!({"source_id": src, "target_id": dst}))
                .unwrap();
            post_webhook(&app, path, &body, Some(&sign(secret, &body)), None).await
        }
    };
    let (st, _) = send(
        "/api/upstream/person-merged",
        b"psecret",
        old_person,
        new_person,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = send(
        "/api/upstream/worker-merged",
        b"wsecret",
        old_worker,
        new_worker,
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let row = db
        .one(Stmt::new(
            "SELECT person_id, worker_id FROM appointments WHERE id = $1",
            [appt.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.col::<Uuid>("person_id").unwrap(), new_person);
    assert_eq!(row.col::<Uuid>("worker_id").unwrap(), new_worker);
    for action in ["upstream_person_merged", "upstream_worker_merged"] {
        let n = db
            .count(Stmt::new(
                "SELECT count(*) AS n FROM audit_log WHERE action = $1 AND entity_id = ANY($2)",
                [action.into(), vec![old_person, old_worker].into()],
            ))
            .await
            .unwrap();
        assert!(n >= 1, "{action} is audited");
    }
    unsafe {
        std::env::remove_var("PERSON_SERVICE_WEBHOOK_SECRET");
        std::env::remove_var("WORKER_SERVICE_WEBHOOK_SECRET");
    }
}
