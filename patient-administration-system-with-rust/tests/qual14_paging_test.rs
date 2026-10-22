//! QUAL-14: lists refuse a limit outside the range and say when they were cut short. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::referrals::NewReferral;
use tower::ServiceExt;
use uuid::Uuid;

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_list_says_whether_it_was_cut_short() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    for _ in 0..3 {
        s.referrals
            .create(
                NewReferral {
                    person_id: Uuid::new_v4(),
                    source: "gp".into(),
                    external_ref: None,
                    target_service: service.clone(),
                    reason: None,
                    referring_worker_id: None,
                    received_at: None,
                },
                &UserContext::system(),
            )
            .await
            .unwrap();
    }
    let app = patient_administration_system::api::rest::router(s.clone());
    let base = format!("/api/referrals?service={service}");

    // Exactly as many as exist: not truncated.
    let (st, v) = get(&app, &format!("{base}&limit=3")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"].as_array().unwrap().len(), 3);
    assert_eq!(v["meta"]["limit"], 3);
    assert_eq!(v["meta"]["returned"], 3);
    assert_eq!(v["meta"]["truncated"], false);

    // Fewer than exist: cut, and it says so.
    let (st, v) = get(&app, &format!("{base}&limit=2")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
    assert_eq!(v["meta"]["returned"], 2);
    assert_eq!(v["meta"]["truncated"], true);

    // More room than needed: the default applies when nothing is asked.
    let (_, v) = get(&app, &base).await;
    assert_eq!(v["meta"]["limit"], 100);
    assert_eq!(v["meta"]["truncated"], false);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_limit_outside_the_range_is_a_400_not_a_silent_change() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    // Every list endpoint that takes ?limit=.
    for path in [
        "/api/referrals",
        "/api/support-flags/open",
        "/api/follow-up-tasks/open",
        "/api/staff-call-tasks/open",
        "/api/inbound-texts/open",
        "/api/conversions/pending",
        "/api/contact-problems",
        "/api/print-queue",
        "/api/audit/recent",
        "/api/admin/outbox/dead-letters",
    ] {
        for bad in ["0", "-1", "501", "99999999999"] {
            let (st, v) = get(&app, &format!("{path}?limit={bad}")).await;
            assert_eq!(st, StatusCode::BAD_REQUEST, "{path}?limit={bad}: {v}");
            assert!(
                v["error"]["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("limit must be between 1 and 500"),
                "{path}?limit={bad}: {v}"
            );
        }
        let (st, v) = get(&app, &format!("{path}?limit=500")).await;
        assert_ne!(st, StatusCode::BAD_REQUEST, "{path}?limit=500: {v}");
    }
}
