//! FR-226, NFR-23: the staff-calendar range is a signed-cursor list, and its pages make the whole range.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::LINK},
};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::schedule::Slot;
use tower::ServiceExt;
use uuid::Uuid;

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Option<String>, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let link = response
        .headers()
        .get(LINK)
        .map(|value| value.to_str().unwrap().to_string());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, link, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_busy_window_has_signed_pages_whose_rows_make_the_whole_window() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&state.db).await;
    // Far beyond the development fixtures (and past the 4 000 to 24 000 days the report tests use), at a random
    // second of about 31 years, so a long-used shared database cannot add unrelated rows to the two-page window.
    // A spread of 1 000 000 seconds (11 days) let runs collide after a few dozen.
    let random_seconds = i64::try_from(Uuid::new_v4().as_u128() % 1_000_000_000).unwrap();
    let start = jiff::Timestamp::now()
        + patient_administration_system::time::days(30_000)
        + jiff::SignedDuration::from_secs(random_seconds);
    let first_slot = SlotRepository::create(
        &state.db,
        &Slot::new(
            schedule.id,
            start,
            start + jiff::SignedDuration::from_mins(30),
        ),
    )
    .await
    .unwrap();
    let second_slot = SlotRepository::create(
        &state.db,
        &Slot::new(
            schedule.id,
            start + jiff::SignedDuration::from_hours(1),
            start + jiff::SignedDuration::from_mins(90),
        ),
    )
    .await
    .unwrap();
    for slot in [&first_slot, &second_slot] {
        state
            .scheduling
            .book_slot(slot.id, Uuid::new_v4(), &UserContext::system())
            .await
            .unwrap();
    }
    let app = patient_administration_system::api::rest::router(state);
    let from = first_slot.start_datetime.strftime("%Y-%m-%dT%H:%M:%S%.6fZ");
    let to = (second_slot.end_datetime + jiff::SignedDuration::from_secs(1))
        .strftime("%Y-%m-%dT%H:%M:%S%.6fZ");
    let first_uri = format!("/api/appointments?from={from}&to={to}&limit=1");

    let (status, link, first) = get(&app, &first_uri).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["meta"]["limit"], 1);
    assert_eq!(first["meta"]["returned"], 1);
    assert_eq!(first["meta"]["truncated"], true);
    let next = first["meta"]["next"].as_str().unwrap();
    assert_eq!(link, Some(format!("<{next}>; rel=\"next\"")));

    let (status, link, second) = get(&app, next).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["meta"]["returned"], 1);
    assert_eq!(second["meta"]["truncated"], false);
    assert!(link.is_none());

    let first_id = first["data"][0]["id"].as_str().unwrap();
    let second_id = second["data"][0]["id"].as_str().unwrap();
    assert_ne!(first_id, second_id);

    let cursor = next.split("cursor=").nth(1).unwrap();
    let changed_filter = format!(
        "/api/appointments?from={from}&to={}&limit=1&cursor={cursor}",
        (second_slot.start_datetime + patient_administration_system::time::days(2))
            .strftime("%Y-%m-%dT%H:%M:%SZ")
    );
    let (status, _, body) = get(&app, &changed_filter).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    let (status, _, body) = get(
        &app,
        &format!("/api/appointments?from={from}&to={to}&limit=501"),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}
