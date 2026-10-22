//! NEW-15: follow-up target dates and the delayed-follow-up report. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! The report hides cells under k and other runs may have left entries behind, so this works in
//! differences: it adds ten unbooked entries 100% past their target and ten booked ones on time, and checks
//! the published total rose by twenty and both cells are counted apart. The exact percentages are unit
//! tests in `follow_up_delay`.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::follow_up_delay::{DelayReport, FollowUpDelayService};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn cell(r: &DelayReport, booked: bool, band: &str) -> Option<i64> {
    r.cells
        .iter()
        .find(|c| {
            c.dims.contains(&("booked".to_string(), booked.to_string()))
                && c.dims.contains(&("band".to_string(), band.to_string()))
        })
        .and_then(|c| c.count)
}

/// A waiting-list entry added `days_ago` days ago, waiting or booked.
async fn entry(db: &sea_orm::DatabaseConnection, days_ago: i64, booked: bool) -> Uuid {
    let id = Uuid::new_v4();
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO waitlist_entries (id, person_id, target_service, priority, status, created_at, updated_at) \
         VALUES ($1, $2, 'new15', 'routine', $3, now() - make_interval(days => $4), now())",
        [
            id.into(),
            Uuid::new_v4().into(),
            (if booked { "booked" } else { "waiting" }).into(),
            (days_ago as i32).into(),
        ],
    ))
    .await
    .unwrap();
    id
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn overdue_and_on_time_follow_ups_are_counted_apart_booked_and_not() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = FollowUpDelayService::new(state.db.clone());
    let tz = patient_administration_system::inbound::messages::timezone();
    let today = Utc::now().with_timezone(&tz).date_naive();
    let before = svc.report(today, 10).await.unwrap();

    // Ten waited eight weeks against a four-week target: 100% delayed, not booked.
    for _ in 0..10 {
        let id = entry(&state.db, 56, false).await;
        svc.set_target(
            id,
            today - Duration::days(28),
            "Clinic coordinator",
            &UserContext::system(),
        )
        .await
        .unwrap();
    }
    // Ten booked and not yet due.
    for _ in 0..10 {
        let id = entry(&state.db, 0, true).await;
        svc.set_target(
            id,
            today + Duration::days(28),
            "Clinic coordinator",
            &UserContext::system(),
        )
        .await
        .unwrap();
    }

    let after = svc.report(today, 10).await.unwrap();
    assert_eq!(
        after.total - before.total,
        20,
        "every entry with a target is counted once"
    );
    assert!(
        cell(&after, false, "100_or_more").is_some_and(|n| n >= 10),
        "{:?}",
        after.cells
    );
    assert!(
        cell(&after, true, "on_time").is_some_and(|n| n >= 10),
        "{:?}",
        after.cells
    );
    // No identifier in the report.
    let text = serde_json::to_string(&after).unwrap();
    assert!(
        !text.contains("person_id") && !text.contains("\"id\""),
        "{text}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn setting_a_target_needs_a_named_person_and_is_audited() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let id = entry(&state.db, 3, false).await;
    let app = patient_administration_system::api::rest::router(state.clone());
    let put = |body: serde_json::Value| {
        Request::builder()
            .method("PUT")
            .uri(format!("/api/waitlist/{id}/follow-up-target"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let target = (Utc::now() + Duration::days(30)).date_naive();

    let refused = app
        .clone()
        .oneshot(put(
            serde_json::json!({ "target_date": target, "set_by": "  " }),
        ))
        .await
        .unwrap();
    assert!(refused.status().is_client_error(), "{}", refused.status());
    let row = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT follow_up_target IS NULL AS unset FROM waitlist_entries WHERE id = $1",
            [id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert!(
        row.try_get::<bool>("", "unset").unwrap(),
        "a refused request changes nothing"
    );

    let ok = app
        .oneshot(put(
            serde_json::json!({ "target_date": target, "set_by": "Clinic coordinator" }),
        ))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let audit = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = 'set_follow_up_target'",
            [id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audit.try_get::<i64>("", "n").unwrap(), 1);

    // A removed or treated entry has no follow-up to date.
    state
        .db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE waitlist_entries SET status = 'treated' WHERE id = $1",
            [id.into()],
        ))
        .await
        .unwrap();
    let svc = FollowUpDelayService::new(state.db.clone());
    assert!(
        svc.set_target(id, target, "Clinic coordinator", &UserContext::system())
            .await
            .is_err()
    );
}
