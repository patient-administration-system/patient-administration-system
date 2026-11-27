//! TSC-12: the same change to the same record sent again and again is refused, so a write-back loop is seen.
//! The default is 10 in 60 seconds (`PAS_LOOP_LIMIT`, `PAS_LOOP_WINDOW_SECONDS`). Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header::RETRY_AFTER},
};
use tower::ServiceExt;
use uuid::Uuid;

async fn post(
    app: &axum::Router,
    uri: &str,
    caller: &str,
    body: &str,
) -> (StatusCode, Option<String>) {
    let r = app
        .clone()
        .oneshot(
            Request::post(uri)
                .header("content-type", "application/json")
                .header("x-user-id", caller)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let retry = r
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (r.status(), retry)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_same_change_is_refused_after_the_limit_and_other_changes_are_not() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    // Two patients with recorded needs, so a review is a change that is made (only those are counted).
    let (person, other_person) = (Uuid::new_v4(), Uuid::new_v4());
    for id in [person, other_person] {
        use patient_administration_system::db::sql::{Conn, Stmt};
        state
            .db
            .run(Stmt::new(
                "INSERT INTO patient_communication_profiles (person_id, accessibility_needs) VALUES ($1, $2)",
                [id.into(), vec!["large_print".to_string()].into()],
            ))
            .await
            .unwrap();
    }
    let app = patient_administration_system::api::rest::router(state);
    let caller = format!("integration-{}", Uuid::new_v4());
    let record = format!("/api/patients/{person}/communication-needs/review");
    let change = r#"{"reviewed_by":"Loop"}"#;

    // A refused request is not a change made: these do not count towards the limit.
    for _ in 0..15 {
        let (status, _) = post(&app, &record, &caller, r#"{"reviewed_by":" "}"#).await;
        assert!(
            status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS,
            "{status}"
        );
    }
    for n in 1..=10 {
        let (status, _) = post(&app, &record, &caller, change).await;
        assert_eq!(status, StatusCode::OK, "request {n} is under the limit");
    }
    let (status, retry) = post(&app, &record, &caller, change).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "the eleventh is refused"
    );
    let wait: u64 = retry.expect("Retry-After").parse().unwrap();
    assert!((1..=60).contains(&wait), "{wait}");

    // A different change, another record and another caller are each counted apart.
    let (status, _) = post(&app, &record, &caller, r#"{"reviewed_by":"Someone else"}"#).await;
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    let other = format!("/api/patients/{other_person}/communication-needs/review");
    let (status, _) = post(&app, &other, &caller, change).await;
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    let (status, _) = post(&app, &record, &format!("{caller}-b"), change).await;
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);

    // Reads are never counted.
    for _ in 0..20 {
        let r = app
            .clone()
            .oneshot(
                Request::get("/api/communication-needs/due-review?limit=1")
                    .header("x-user-id", &caller)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
