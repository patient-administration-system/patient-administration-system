//! TSC-6: the main lists say what changed in a window, for an integrator that polls rather than subscribes.
//! Each list gets rows whose `updated_at` is set inside a window in 2001, which no other test's rows reach, and
//! one just before it; the window is read a row a page, so the walk crosses pages. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::db::sql::{Conn, Stmt};
use tower::ServiceExt;
use uuid::Uuid;

/// A day of its own for each run, in the past where no other test's rows are, so a long-used database does not
/// add the rows of an earlier run to this one's window.
fn window() -> (String, String) {
    let day = Uuid::new_v4().as_u128() % 10_000;
    let since = jiff::Timestamp::from_second(978_307_200).unwrap()
        + jiff::SignedDuration::from_hours(i64::try_from(day).unwrap() * 24);
    let before = since + jiff::SignedDuration::from_hours(24);
    (since.to_string(), before.to_string())
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let body = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

/// Every id on every page from `first`, following `meta.next`.
async fn walk(app: &axum::Router, first: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut uri = Some(first.to_string());
    let mut pages = 0;
    while let Some(u) = uri.take() {
        let (status, body) = get(app, &u).await;
        assert_eq!(status, StatusCode::OK, "{u}: {body}");
        for row in body["data"].as_array().unwrap() {
            ids.push(row["id"].as_str().unwrap().to_string());
        }
        uri = body["meta"]["next"].as_str().map(str::to_string);
        pages += 1;
        assert!(pages < 100, "the pages end");
    }
    ids
}

/// Set a row's `updated_at` to an offset in seconds from `since`, the start of the window (negative is before it).
async fn changed_at(
    db: &patient_administration_system::db::sql::Pool,
    table: &str,
    since: &str,
    id: Uuid,
    offset_secs: i64,
) {
    db.run(Stmt::new(
        format!("UPDATE {table} SET updated_at = $2::timestamptz + make_interval(secs => $3) WHERE id = $1"),
        [id.into(), since.into(), offset_secs.into()],
    ))
    .await
    .unwrap();
}

/// Rows of `table` before, inside (twice, the later one first in creation order) and after the window, and
/// what a walk of the window must find: the two inside, oldest change first.
async fn spread(
    db: &patient_administration_system::db::sql::Pool,
    table: &str,
    since: &str,
    ids: [Uuid; 4],
) -> Vec<String> {
    let [before, late, early, after] = ids;
    changed_at(db, table, since, before, -1).await;
    changed_at(db, table, since, late, 7_200).await;
    changed_at(db, table, since, early, 3_600).await;
    changed_at(db, table, since, after, 86_400).await;
    vec![early.to_string(), late.to_string()]
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn each_list_returns_only_what_changed_in_the_window_oldest_change_first_across_pages() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let (since, before) = window();
    let window = format!("updated_since={since}&updated_before={before}&limit=1");

    // Appointments.
    let mut appts = [Uuid::nil(); 4];
    for id in &mut appts {
        *id = Uuid::new_v4();
        s.db.run(Stmt::new(
            "INSERT INTO appointments (id, person_id, start_datetime, end_datetime, status, created_at, updated_at) \
             VALUES ($1, $2, now(), now() + interval '30 minutes', 'booked', now(), now())",
            [(*id).into(), Uuid::new_v4().into()],
        ))
        .await
        .unwrap();
    }
    let expected = spread(&s.db, "appointments", &since, appts).await;
    assert_eq!(
        walk(&app, &format!("/api/appointments?{window}")).await,
        expected
    );

    // Referrals.
    let service = format!("svc-{}", Uuid::new_v4());
    let mut refs = [Uuid::nil(); 4];
    for id in &mut refs {
        let (r, _) = s
            .referrals
            .create(
                patient_administration_system::referrals::NewReferral {
                    person_id: Uuid::new_v4(),
                    source: "internal".into(),
                    external_ref: None,
                    target_service: service.clone(),
                    reason: None,
                    referring_worker_id: None,
                    received_at: None,
                },
                &patient_administration_system::db::repositories::audit::UserContext::system(),
            )
            .await
            .unwrap();
        *id = r.id;
    }
    let expected = spread(&s.db, "referrals", &since, refs).await;
    assert_eq!(
        walk(&app, &format!("/api/referrals?{window}")).await,
        expected
    );
    assert_eq!(
        walk(&app, &format!("/api/referrals?service={service}&{window}")).await,
        expected,
        "with a filter"
    );

    // Waiting-list entries, of every service.
    let mut entries = [Uuid::nil(); 4];
    for id in &mut entries {
        *id = Uuid::new_v4();
        s.db.run(Stmt::new(
            "INSERT INTO waitlist_entries (id, person_id, target_service, priority, status, created_at, updated_at) \
             VALUES ($1, $2, $3, 'routine', 'waiting', now(), now())",
            [(*id).into(), Uuid::new_v4().into(), service.clone().into()],
        ))
        .await
        .unwrap();
    }
    let expected = spread(&s.db, "waitlist_entries", &since, entries).await;
    assert_eq!(
        walk(&app, &format!("/api/waitlist?{window}")).await,
        expected
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_window_that_cannot_be_walked_is_refused_and_the_old_lists_are_unchanged() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let (since, before) = window();
    for uri in [
        format!("/api/appointments?updated_before={before}"),
        format!("/api/referrals?updated_since={before}&updated_before={since}"),
        "/api/appointments".to_string(),
        "/api/waitlist".to_string(),
        format!("/api/appointments?from={since}"),
    ] {
        let (status, body) = get(&app, &uri).await;
        assert!(status.is_client_error(), "{uri}: {status} {body}");
    }
    // A cursor holds its window: another window refuses it.
    let (_, first) = get(
        &app,
        "/api/referrals?updated_since=2000-01-01T00:00:00Z&limit=1",
    )
    .await;
    if let Some(next) = first["meta"]["next"].as_str() {
        let moved = next.replace("2000-01-01", "1999-01-01");
        let (status, _) = get(&app, &moved).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{moved}");
    }
    let (status, _) = get(&app, &format!("/api/appointments?from={since}&to={before}")).await;
    assert_eq!(status, StatusCode::OK, "a window of starts still works");
    let (status, _) = get(&app, "/api/waitlist?service=cardiology").await;
    assert_eq!(status, StatusCode::OK, "a service's list still works");
}
