//! TSC-7: saved views of the work queue. A view returns the same items as its filter; it is shared by role,
//! and saving, sharing and deleting it are audited. Needs PostgreSQL: `cargo test -- --ignored --test-threads=1`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jiff::Timestamp;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::sql::{Conn, Pool, Stmt};
use patient_administration_system::work_queue::{NewView, WorkKind, WorkQueueService};
use tower::ServiceExt;
use uuid::Uuid;

fn ctx(who: &str) -> UserContext {
    UserContext {
        user_id: Some(who.to_string()),
        user_ip: None,
        user_agent: None,
    }
}

async fn call_task(db: &Pool, reason: &str) -> Uuid {
    let id = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) VALUES ($1,$2,$3,$4, now() - interval '1 hour')",
        [id.into(), Uuid::new_v4().into(), Uuid::new_v4().into(), reason.into()],
    ))
    .await
    .unwrap();
    id
}

async fn get(app: &axum::Router, uri: &str, who: &str) -> (StatusCode, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(
            Request::get(uri)
                .header("x-user-id", who)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let b = to_bytes(r.into_body(), 4 << 20).await.unwrap();
    (status, serde_json::from_slice(&b).unwrap_or_default())
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_saved_view_returns_the_same_items_as_its_filter() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let db = state.db.clone();
    let svc = WorkQueueService::new(db.clone());
    call_task(&db, "urgent_pathway").await;
    call_task(&db, "silent").await;
    svc.sync(Timestamp::now()).await.unwrap();
    let who = format!("Ann-{}", Uuid::new_v4());
    let view = svc
        .save_view(
            &who,
            NewView {
                name: "Urgent calls, overdue".into(),
                queue: Some("booking".into()),
                owner: Some("none".into()),
                priority: Some(1),
                overdue: true,
                kind: Some(WorkKind::Call),
            },
            &ctx(&who),
        )
        .await
        .unwrap();
    assert_eq!(common::audit_count(&db, view.id, "create").await, 1);
    let app = patient_administration_system::api::rest::router(state);
    let (st, direct) = get(
        &app,
        "/api/work-items?queue=booking&owner=none&priority=1&overdue=true&kind=call&limit=200",
        &who,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{direct}");
    let (st, saved) = get(
        &app,
        &format!("/api/work-views/{}/items?limit=200", view.id),
        &who,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{saved}");
    assert!(!ids(&direct).is_empty());
    assert_eq!(ids(&saved), ids(&direct));
    assert!(
        saved["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["priority"] == 1 && i["kind"] == "call")
    );

    // A name once per person, a known queue, a priority from 1 to 3.
    let again = svc
        .save_view(
            &who,
            NewView {
                name: "Urgent calls, overdue".into(),
                ..NewView::default()
            },
            &ctx(&who),
        )
        .await;
    assert!(matches!(
        again,
        Err(patient_administration_system::Error::Conflict(_))
    ));
    assert!(
        svc.save_view(
            &who,
            NewView {
                name: "x".into(),
                queue: Some("nowhere".into()),
                ..NewView::default()
            },
            &ctx(&who)
        )
        .await
        .is_err()
    );
    assert!(
        svc.save_view(
            &who,
            NewView {
                name: "y".into(),
                priority: Some(4),
                ..NewView::default()
            },
            &ctx(&who)
        )
        .await
        .is_err()
    );
    let (st, _) = get(&app, "/api/work-items?priority=9", &who).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_view_is_seen_by_its_maker_and_by_the_role_it_is_shared_with_and_only_its_maker_changes_it()
 {
    let db = common::build_state(&common::database_url().unwrap())
        .await
        .db;
    let svc = WorkQueueService::new(db.clone());
    let (ann, bob) = (
        format!("Ann-{}", Uuid::new_v4()),
        format!("Bob-{}", Uuid::new_v4()),
    );
    let role = format!("booking-team-{}", Uuid::new_v4());
    let mine = svc
        .save_view(
            &ann,
            NewView {
                name: "Mine".into(),
                owner: Some("me".into()),
                ..NewView::default()
            },
            &ctx(&ann),
        )
        .await
        .unwrap();
    let roles = vec![role.clone()];
    assert!(
        svc.view(mine.id, &bob, &roles).await.is_err(),
        "not shared yet"
    );
    assert!(
        svc.views_for(&bob, &roles)
            .await
            .unwrap()
            .iter()
            .all(|v| v.id != mine.id)
    );

    assert!(
        matches!(
            svc.share_view(mine.id, &bob, Some(&role), &ctx(&bob)).await,
            Err(patient_administration_system::Error::Conflict(_))
        ),
        "only its maker shares it"
    );
    let shared = svc
        .share_view(mine.id, &ann, Some(&role), &ctx(&ann))
        .await
        .unwrap();
    assert_eq!(shared.shared_with_role.as_deref(), Some(role.as_str()));
    assert_eq!(common::audit_count(&db, mine.id, "share").await, 1);
    assert!(
        svc.views_for(&bob, &roles)
            .await
            .unwrap()
            .iter()
            .any(|v| v.id == mine.id)
    );
    assert!(
        svc.view(mine.id, &bob, &[]).await.is_err(),
        "another role does not see it"
    );
    // "me" is whoever looks: Bob sees his own items through Ann's view.
    assert_eq!(
        svc.view(mine.id, &bob, &roles)
            .await
            .unwrap()
            .filter_for(&bob)
            .owner
            .as_deref(),
        Some(bob.as_str())
    );

    assert!(
        svc.delete_view(mine.id, &bob, &ctx(&bob)).await.is_err(),
        "only its maker deletes it"
    );
    svc.share_view(mine.id, &ann, None, &ctx(&ann))
        .await
        .unwrap();
    assert_eq!(common::audit_count(&db, mine.id, "unshare").await, 1);
    assert!(svc.view(mine.id, &bob, &roles).await.is_err());
    svc.delete_view(mine.id, &ann, &ctx(&ann)).await.unwrap();
    assert_eq!(common::audit_count(&db, mine.id, "delete").await, 1);
    assert!(svc.view(mine.id, &ann, &[]).await.is_err());
}
