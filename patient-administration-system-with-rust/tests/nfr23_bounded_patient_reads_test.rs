//! Audit PA-14 (NFR-23): a patient's appointments and portal letters are read a page at a time. Every item is
//! reachable once by following `meta.next`, and the visits still to come are listed on their own, so a long
//! history cannot hide them. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::communication::LetterTemplate;
use std::collections::HashSet;
use tower::ServiceExt;
use uuid::Uuid;

async fn get(app: &axum::Router, uri: &str) -> serde_json::Value {
    let r = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK, "{uri}");
    serde_json::from_slice(&to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap()
}

/// Every row of a paged list, following `meta.next`, with the size of each page.
async fn every_page(app: &axum::Router, first: &str) -> (Vec<serde_json::Value>, Vec<usize>) {
    let (mut rows, mut sizes) = (Vec::new(), Vec::new());
    let mut uri = first.to_string();
    loop {
        let body = get(app, &uri).await;
        let page = body["data"].as_array().unwrap().clone();
        sizes.push(page.len());
        rows.extend(page);
        match body["meta"]["next"].as_str() {
            Some(next) => uri = next.to_string(),
            None => break,
        }
        assert!(sizes.len() < 50, "the pages end");
    }
    (rows, sizes)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_long_history_is_read_a_page_at_a_time_and_upcoming_visits_stay_in_view() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let person = Uuid::new_v4();
    // 130 past visits, one a day, and 4 still to come.
    for (offset, count) in [(-131, 130), (1, 4)] {
        state
            .db
            .run(Stmt::new(
                "INSERT INTO appointments (id, person_id, start_datetime, end_datetime, status, created_at, updated_at) \
                 SELECT gen_random_uuid(), $1, now() + make_interval(days => $2 + g), \
                        now() + make_interval(days => $2 + g, mins => 30), 'fulfilled', now(), now() \
                 FROM generate_series(0, $3 - 1) g",
                [person.into(), offset.into(), count.into()],
            ))
            .await
            .unwrap();
    }
    let app = patient_administration_system::api::rest::router(state.clone());
    let base = format!("/api/patients/{person}/appointments");

    let upcoming = get(&app, &format!("{base}?when=upcoming")).await;
    let soon = upcoming["data"].as_array().unwrap();
    assert_eq!(soon.len(), 4, "the visits to come are on the first page");
    assert_eq!(upcoming["meta"]["truncated"], false);
    let starts: Vec<&str> = soon
        .iter()
        .map(|a| a["start_datetime"].as_str().unwrap())
        .collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]), "soonest first");

    let (past, sizes) = every_page(&app, &format!("{base}?when=past&limit=50")).await;
    assert_eq!(sizes, vec![50, 50, 30], "bounded pages");
    let ids: HashSet<&str> = past.iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 130, "every past visit once");
    let starts: Vec<&str> = past
        .iter()
        .map(|a| a["start_datetime"].as_str().unwrap())
        .collect();
    assert!(starts.windows(2).all(|w| w[0] >= w[1]), "most recent first");

    let (all, sizes) = every_page(&app, &base).await;
    assert_eq!(sizes[0], 100, "the default page is bounded");
    let ids: HashSet<&str> = all.iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 134, "everything once");

    let bad = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{base}?when=someday"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn portal_letters_are_read_a_page_at_a_time_and_keep_their_channel() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let person = Uuid::new_v4();
    let template = LetterRepository::create_template(
        &state.db,
        &LetterTemplate::new(format!("pa14-{}", Uuid::new_v4()), "S".into(), "B".into()),
    )
    .await
    .unwrap()
    .id;
    for (channel, count) in [("portal", 70), ("sms", 5)] {
        state
            .db
            .run(Stmt::new(
                "INSERT INTO generated_letters (id, template_id, person_id, rendered_subject, rendered_body, channel, status, created_at, updated_at) \
                 SELECT gen_random_uuid(), $1, $2, 'Subject', 'Body', $3, 'sent', now() - make_interval(mins => g), now() \
                 FROM generate_series(1, $4) g",
                [template.into(), person.into(), channel.into(), count.into()],
            ))
            .await
            .unwrap();
    }
    let app = patient_administration_system::api::rest::router(state);
    let (letters, sizes) = every_page(
        &app,
        &format!("/api/patients/{person}/letters?channel=portal"),
    )
    .await;
    assert_eq!(sizes, vec![50, 20], "bounded pages (default 50)");
    assert!(
        letters.iter().all(|l| l["channel"] == "portal"),
        "the channel holds across pages"
    );
    let ids: HashSet<&str> = letters.iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 70, "every portal letter once");
    let at: Vec<&str> = letters
        .iter()
        .map(|l| l["created_at"].as_str().unwrap())
        .collect();
    assert!(at.windows(2).all(|w| w[0] >= w[1]), "newest first");
}
