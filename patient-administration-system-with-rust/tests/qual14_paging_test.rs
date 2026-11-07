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

/// QUAL-48: three lists that stopped at a fixed size without saying so.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn templates_experiments_and_due_checks_say_when_they_were_cut() {
    use patient_administration_system::clinic_templates::TemplateService;
    use patient_administration_system::db::sql::{Conn, Stmt};

    let s = common::build_state(&common::database_url().unwrap()).await;
    let ctx = UserContext::system();
    let schedule = common::new_schedule(&s.db).await.id;
    let templates = TemplateService::new(s.db.clone());
    for name in ["Monday clinic", "Friday clinic"] {
        templates
            .create(schedule, name, "Sister Hughes", &ctx)
            .await
            .unwrap();
    }
    let tag = Uuid::new_v4().to_string();
    s.db.run(Stmt::new(
        "INSERT INTO message_experiments (id, template_name, started_by) \
         SELECT gen_random_uuid(), $1 || n, 'Dr A' FROM generate_series(1, 2) n",
        [tag.clone().into()],
    ))
    .await
    .unwrap();
    let service = format!("qual48-{tag}");
    s.db.run(Stmt::new(
        "WITH new AS (INSERT INTO rtt_pathways (id, person_id, target_service, started_at, created_at, updated_at) \
           SELECT gen_random_uuid(), gen_random_uuid(), $1, now() - interval '2 years', now(), now() \
           FROM generate_series(1, 2) RETURNING id, started_at) \
         INSERT INTO rtt_clock_events (id, pathway_id, kind, event_at, created_at) \
         SELECT gen_random_uuid(), id, 'started', started_at, now() FROM new",
        [service.clone().into()],
    ))
    .await
    .unwrap();

    let app = patient_administration_system::api::rest::router(s.clone());
    let templates = format!("/api/session-templates?schedule_id={schedule}");
    let (_, all) = get(&app, &format!("{templates}&limit=2")).await;
    let (_, cut) = get(&app, &format!("{templates}&limit=1")).await;
    let (_, experiments) = get(&app, "/api/experiments?limit=1").await;
    let (_, due) = get(&app, "/api/pathways/due-check?weeks=26&limit=1").await;

    s.db.run(Stmt::new(
        "DELETE FROM rtt_clock_events WHERE pathway_id IN \
           (SELECT id FROM rtt_pathways WHERE target_service = $1)",
        [service.clone().into()],
    ))
    .await
    .unwrap();
    s.db.run(Stmt::new(
        "DELETE FROM rtt_pathways WHERE target_service = $1",
        [service.into()],
    ))
    .await
    .unwrap();

    assert_eq!(all["meta"]["truncated"], false, "{all}");
    assert_eq!(cut["meta"]["truncated"], true, "{cut}");
    assert_eq!(
        cut["data"][0]["name"], "Monday clinic",
        "oldest first: {cut}"
    );
    assert_eq!(experiments["meta"]["truncated"], true, "{experiments}");
    assert_eq!(experiments["data"].as_array().unwrap().len(), 1);
    assert_eq!(due["meta"]["truncated"], true, "{due}");
    assert_eq!(due["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
/// A bad limit is a 422, as axa answers it, on the lists with a next page and on those without.
async fn a_limit_outside_the_range_is_a_422_not_a_silent_change() {
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
        "/api/slot-releases",
        "/api/tolerance-alerts",
        "/api/tolerances/status",
        "/api/session-templates",
        "/api/experiments",
        "/api/pathways/due-check",
    ] {
        for bad in ["0", "-1", "501", "99999999999", "abc"] {
            let (st, v) = get(&app, &format!("{path}?limit={bad}")).await;
            assert_eq!(
                st,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{path}?limit={bad}: {v}"
            );
            assert_eq!(v["code"], "unprocessable_entity", "{path}?limit={bad}: {v}");
            assert!(
                v["detail"]
                    .as_str()
                    .unwrap_or("")
                    .contains("limit must be a whole number from 1 to 500"),
                "{path}?limit={bad}: {v}"
            );
        }
        let (st, v) = get(&app, &format!("{path}?limit=500")).await;
        assert_eq!(st, StatusCode::OK, "{path}?limit=500: {v}");
    }
}
