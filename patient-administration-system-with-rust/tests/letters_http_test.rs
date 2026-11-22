//! The letter handlers over HTTP: a template is stored and read, a letter is previewed and generated for a
//! person, read back, listed by channel and marked sent or failed; the refusals are 4xx. Needs PostgreSQL:
//! `cargo test -- --ignored`.
mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::upstream::{Person, StubPersonClient};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
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

/// An inactive template (it needs no other language) of this name, as a request body.
fn template_body(name: &str) -> Value {
    serde_json::json!({
        "id": Uuid::nil(), "name": name, "subject": "Hello", "body_tera": "Hello there",
        "channels": ["sms"], "required_variables": [], "active": false, "locale": "en-001",
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
    })
}

async fn app_with_person() -> (axum::Router, Uuid) {
    let people = Arc::new(StubPersonClient::new());
    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        serde_json::json!({ "given": "Ada", "family": "Lovelace" }),
    );
    data.insert(
        "telecom".into(),
        serde_json::json!([{ "system": "phone", "value": "+447700900123" }]),
    );
    people.insert(Person { id: person, data });
    let state = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(people);
    (
        patient_administration_system::api::rest::router(state),
        person,
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_template_is_stored_read_and_the_bad_ones_are_refused() {
    let (app, _) = app_with_person().await;
    let name = format!("http-{}", Uuid::new_v4().simple());
    let template = template_body(&name);
    let (st, v) = call(
        &app,
        "POST",
        "/api/letter-templates",
        Some(template.clone()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let tid = v["data"]["id"].as_str().unwrap().to_string();

    let mut bad = template.clone();
    bad["body_tera"] = serde_json::json!("{{ oops");
    let (st, _) = call(&app, "POST", "/api/letter-templates", Some(bad)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let mut bad = template.clone();
    bad["locale"] = serde_json::json!("EN_gb");
    let (st, _) = call(&app, "POST", "/api/letter-templates", Some(bad)).await;
    assert!(st.is_client_error(), "a locale that is not shaped like one");
    let mut lone = template_body(&format!("lone-{}", Uuid::new_v4().simple()));
    lone["active"] = serde_json::json!(true);
    let (st, _) = call(&app, "POST", "/api/letter-templates", Some(lone)).await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "an active template in one language only"
    );

    let (st, v) = call(&app, "GET", &format!("/api/letter-templates/{tid}"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _) = call(
        &app,
        "GET",
        &format!("/api/letter-templates/{}", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = call(&app, "GET", "/api/letter-templates", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(
        v["data"].is_array(),
        "the list shows the active templates; this one is inactive"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_letter_is_previewed_generated_read_listed_and_marked() {
    let (app, person) = app_with_person().await;
    let (_, v) = call(
        &app,
        "POST",
        "/api/letter-templates",
        Some(template_body(&format!("http-{}", Uuid::new_v4().simple()))),
    )
    .await;
    let tid = v["data"]["id"].as_str().unwrap().to_string();

    // Preview renders without storing; an unknown template or person is refused.
    let preview = serde_json::json!({ "template_id": tid, "person_id": person });
    let (st, v) = call(&app, "POST", "/api/letters/preview", Some(preview)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(
        v["data"]["rendered_body"]
            .as_str()
            .unwrap()
            .starts_with("Hello")
    );
    let (st, _) = call(
        &app,
        "POST",
        "/api/letters/preview",
        Some(serde_json::json!({ "template_id": Uuid::new_v4(), "person_id": person })),
    )
    .await;
    assert!(st.is_client_error());

    let generate = serde_json::json!({ "template_id": tid, "person_id": person, "channel": "sms" });
    let (st, v) = call(
        &app,
        "POST",
        "/api/letters/generate",
        Some(generate.clone()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let lid = v["data"]["id"].as_str().unwrap().to_string();
    let (st, _) = call(
        &app,
        "POST",
        "/api/letters/generate",
        Some(serde_json::json!({ "template_id": Uuid::new_v4(), "person_id": person, "channel": "sms" })),
    )
    .await;
    assert!(st.is_client_error());

    let (st, v) = call(&app, "GET", &format!("/api/letters/{lid}"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _) = call(
        &app,
        "GET",
        &format!("/api/letters/{}", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // A person's letters, all and by channel; a channel that is not one is refused.
    let (st, v) = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/letters"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v.to_string().contains(&lid));
    let (_, v) = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/letters?channel=sms"),
        None,
    )
    .await;
    assert!(v.to_string().contains(&lid));
    let (_, v) = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/letters?channel=email"),
        None,
    )
    .await;
    assert!(!v.to_string().contains(&lid));
    let (st, _) = call(
        &app,
        "GET",
        &format!("/api/patients/{person}/letters?channel=pigeon"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // Marked sent, or failed (a second letter), and an unknown letter is a 404.
    let (st, v) = call(&app, "POST", &format!("/api/letters/{lid}/sent"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, v) = call(&app, "POST", "/api/letters/generate", Some(generate)).await;
    let second = v["data"]["id"].as_str().unwrap().to_string();
    let (st, v) = call(&app, "POST", &format!("/api/letters/{second}/failed"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    for tail in ["sent", "failed"] {
        let (st, _) = call(
            &app,
            "POST",
            &format!("/api/letters/{}/{tail}", Uuid::new_v4()),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{tail}");
    }
}
