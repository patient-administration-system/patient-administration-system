//! The no-show endpoints over HTTP with the score switched off (its default): what answers, what is refused
//! while it is off, and the checks on a review and a stage change. Needs PostgreSQL:
//! `cargo test -- --ignored`.
mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let r = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let st = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn with_the_score_off_the_reads_answer_and_the_scoring_calls_are_refused() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let (st, status) = call(&app, "GET", "/api/noshow/status", None).await;
    assert_eq!(st, StatusCode::OK, "{status}");
    // The rest of this test is about the default: the score off.
    if status["data"]["active"] == true {
        return;
    }
    let (st, card) = call(&app, "GET", "/api/noshow/model-card", None).await;
    assert_eq!(st, StatusCode::OK, "{card}");
    assert!(card["data"]["bands"].is_object());

    let id = Uuid::new_v4();
    for (method, uri) in [
        ("GET", format!("/api/appointments/{id}/noshow")),
        ("POST", "/api/noshow/recalculate".to_string()),
        ("GET", "/api/noshow/priority-list?limit=5".to_string()),
    ] {
        let (st, v) = call(&app, method, &uri, None).await;
        assert_eq!(st, StatusCode::CONFLICT, "{method} {uri}: {v}");
        assert!(v.to_string().contains("PAS_NOSHOW_SCORE"), "{v}");
    }
    let (st, v) = call(
        &app,
        "GET",
        &format!("/api/noshow/for-appointments?ids={id},not-an-id"),
        None,
    )
    .await;
    // An entry that is not an id is refused, naming the list; nothing is read for the others.
    assert!(st.is_client_error(), "{st} {v}");
    assert!(v.to_string().contains("ids"), "{v}");
    let (st, v) = call(
        &app,
        "GET",
        &format!("/api/noshow/for-appointments?ids={id}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["active"], false);
    assert_eq!(v["data"]["items"].as_array().map(Vec::len), Some(0));

    // The audits work while it is off.
    for uri in [
        "/api/noshow/fairness-audit?days=30&k=5",
        "/api/noshow/fairness-audit",
        "/api/noshow/evaluation?days=30",
        "/api/noshow/evaluation",
    ] {
        let (st, v) = call(&app, "GET", uri, None).await;
        assert_eq!(st, StatusCode::OK, "{uri}: {v}");
    }
    let (st, _) = call(&app, "GET", "/api/noshow/rollout", None).await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_review_and_a_stage_change_need_a_named_person_a_known_kind_and_a_reason() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);

    let review = |kind: &str, reviewer: &str, outcome: &str| json!({ "kind": kind, "reviewer": reviewer, "outcome": outcome, "notes": "reviewed in the test" });
    for bad in [
        review("nope", "Dr A", "approved"),
        review("fairness", " ", "approved"),
        review("fairness", "Dr A", "maybe"),
    ] {
        let (st, _) = call(&app, "POST", "/api/noshow/reviews", Some(bad)).await;
        assert!(st.is_client_error());
    }
    for kind in ["fairness", "clinical_safety"] {
        let (st, v) = call(
            &app,
            "POST",
            "/api/noshow/reviews",
            Some(review(kind, "Dr A", "rejected")),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{kind}: {v}");
    }

    let clinic = format!("clinic-{}", Uuid::new_v4().simple());
    let url = format!("/api/noshow/rollout/{clinic}");
    let stage = |stage: &str, by: &str, note: &str| json!({ "stage": stage, "changed_by": by, "note": note });
    let (st, _) = call(&app, "PUT", &url, Some(stage("bogus", "Dr A", "a reason"))).await;
    assert!(st.is_client_error(), "a stage that is neither");
    let (st, _) = call(&app, "PUT", &url, Some(stage("live", "Dr A", "short"))).await;
    assert!(
        st.is_client_error(),
        "going live needs a reason of 20 characters"
    );
    let (st, _) = call(&app, "PUT", &url, Some(stage("shadow", " ", "a reason"))).await;
    assert!(st.is_client_error(), "a blank name");
    let (st, v) = call(
        &app,
        "PUT",
        &url,
        Some(stage("shadow", "Dr A", "start recording")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
}
