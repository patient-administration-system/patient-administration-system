//! Reasonable adjustments over HTTP: record, list (live and ended), the code catalogue, the per-appointment
//! view and ending one; and what is refused. Needs PostgreSQL: `cargo test -- --ignored`.
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
async fn an_adjustment_is_recorded_listed_ended_and_the_bad_requests_are_refused() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let person = Uuid::new_v4();
    let url = format!("/api/patients/{person}/reasonable-adjustments");

    let (st, v) = call(&app, "GET", "/api/reasonable-adjustments/codes", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let codes = v["data"]["codes"].as_array().unwrap();
    assert!(codes.iter().any(|c| c["code"] == "step_free"), "{v}");
    assert!(v["data"]["system"].is_string());

    let body =
        |code: &str, by: &str| json!({ "code": code, "note": "front entrance", "recorded_by": by });
    let (st, _) = call(&app, "POST", &url, Some(body("not_a_code", "Receptionist"))).await;
    assert!(st.is_client_error(), "an unknown code");
    let (st, _) = call(&app, "POST", &url, Some(body("step_free", " "))).await;
    assert!(st.is_client_error(), "a blank name");
    let (st, v) = call(&app, "POST", &url, Some(body("step_free", "Receptionist"))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["data"]["id"].as_str().unwrap().to_string();
    let (st, _) = call(&app, "POST", &url, Some(body("step_free", "Receptionist"))).await;
    assert_eq!(st, StatusCode::CONFLICT, "the same live adjustment twice");

    let (st, v) = call(&app, "GET", &url, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v.to_string().contains(&id));
    let (st, _) = call(&app, "GET", &format!("{url}?limit=lots"), None).await;
    assert!(st.is_client_error());

    // The view for a list of appointments refuses a list with an entry that is not an id, naming the entry.
    let (st, v) = call(
        &app,
        "GET",
        &format!(
            "/api/reasonable-adjustments/for-appointments?ids={},not-an-id,{}",
            Uuid::new_v4(),
            Uuid::new_v4()
        ),
        None,
    )
    .await;
    assert!(st.is_client_error(), "{st} {v}");
    assert!(v.to_string().contains("entry 2"), "{v}");
    let (st, v) = call(
        &app,
        "GET",
        &format!(
            "/api/reasonable-adjustments/for-appointments?ids={},{}",
            Uuid::new_v4(),
            Uuid::new_v4()
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"].as_array().map(Vec::len), Some(0));

    let end = format!("/api/reasonable-adjustments/{id}/end");
    let (st, _) = call(&app, "POST", &end, Some(json!({ "ended_by": " " }))).await;
    assert!(st.is_client_error(), "a blank name");
    let (st, v) = call(
        &app,
        "POST",
        &end,
        Some(json!({ "ended_by": "Receptionist" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["status"], "ended");
    let (st, _) = call(
        &app,
        "POST",
        &end,
        Some(json!({ "ended_by": "Receptionist" })),
    )
    .await;
    assert!(st.is_client_error(), "only once");
    let (st, _) = call(
        &app,
        "POST",
        &format!("/api/reasonable-adjustments/{}/end", Uuid::new_v4()),
        Some(json!({ "ended_by": "Receptionist" })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Ended: gone from the live list, there with `include_ended`.
    let (_, live) = call(&app, "GET", &url, None).await;
    assert!(!live.to_string().contains(&id));
    let (_, all) = call(&app, "GET", &format!("{url}?include_ended=true"), None).await;
    assert!(all.to_string().contains(&id));
}
