//! NEW-11: a consultant's triage of a referral. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::waitlist::Priority;
use patient_administration_system::referrals::NewReferral;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    service: String,
    other: String,
    other_schedule: Uuid,
}

async fn world() -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let other = format!("svc-{}", Uuid::new_v4());
    common::new_schedule_for(&s.db, &service).await;
    let other_schedule = common::new_schedule_for(&s.db, &other).await.id;
    W {
        s,
        service,
        other,
        other_schedule,
    }
}

impl W {
    async fn received(&self, person: Uuid) -> Uuid {
        self.s
            .referrals
            .create(
                NewReferral {
                    person_id: person,
                    source: "gp".into(),
                    external_ref: None,
                    target_service: self.service.clone(),
                    reason: Some("needs a view".into()),
                    referring_worker_id: None,
                    received_at: None,
                },
                &sys(),
            )
            .await
            .unwrap()
            .0
            .id
    }
    async fn audit_rows(&self, id: Uuid) -> i64 {
        self.s
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM audit_log WHERE entity_type = 'referral' AND entity_id = $1 AND action = 'triaged'",
                [id.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "n")
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn returned_with_advice_ends_the_referral_and_it_cannot_be_booked_listed_or_closed() {
    let w = world().await;
    let id = w.received(Uuid::new_v4()).await;
    let r =
        w.s.referrals
            .triage(
                id,
                "returned_with_advice",
                "  Mr R. Consultant ",
                None,
                &sys(),
            )
            .await
            .unwrap();
    assert_eq!(r.status, "returned_with_advice");
    assert_eq!(r.triaged_by.as_deref(), Some("Mr R. Consultant"));
    assert!(r.triaged_at.is_some() && r.redirected_referral_id.is_none());
    assert_eq!(w.audit_rows(id).await, 1);

    assert!(
        w.s.referrals
            .book_first_available(id, &sys())
            .await
            .is_err()
    );
    assert!(
        w.s.referrals
            .waitlist(id, Priority::Routine, &sys())
            .await
            .is_err()
    );
    assert!(w.s.referrals.close(id, None, &sys()).await.is_err());
    assert!(
        w.s.referrals
            .triage(id, "returned_with_advice", "Mr R", None, &sys())
            .await
            .is_err(),
        "triage happens once"
    );
    let listed =
        w.s.referrals
            .list(Some("returned_with_advice"), Some(&w.service), 50)
            .await
            .unwrap();
    assert!(listed.iter().any(|x| x.id == id));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_redirect_creates_the_new_referral_so_the_patient_is_never_lost() {
    let w = world().await;
    let person = Uuid::new_v4();
    let id = w.received(person).await;
    let r =
        w.s.referrals
            .triage(id, "redirected", "Mr R", Some(&w.other), &sys())
            .await
            .unwrap();
    assert_eq!(r.status, "redirected");
    assert_eq!(r.redirected_to_service.as_deref(), Some(w.other.as_str()));
    let new_id = r.redirected_referral_id.expect("the new referral");
    let new = w.s.referrals.get(new_id).await.unwrap();
    assert_eq!(
        (new.person_id, new.status.as_str(), new.source.as_str()),
        (person, "received", "internal")
    );
    assert_eq!(new.target_service, w.other);
    assert_eq!(new.reason.as_deref(), Some("needs a view"));
    // The new referral can be booked from like any other.
    common::free_slot(&w.s.db, w.other_schedule, 3).await;
    assert!(
        w.s.referrals
            .book_first_available(new_id, &sys())
            .await
            .is_ok()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_bad_triage_changes_nothing() {
    let w = world().await;
    let id = w.received(Uuid::new_v4()).await;
    let t = |outcome: &'static str, by: &'static str, to: Option<String>| {
        let (s, id) = (w.s.clone(), id);
        async move {
            s.referrals
                .triage(id, outcome, by, to.as_deref(), &sys())
                .await
        }
    };
    assert!(
        t("returned_with_advice", "  ", None).await.is_err(),
        "no name"
    );
    assert!(
        t("discharged", "Mr R", None).await.is_err(),
        "unknown outcome"
    );
    assert!(t("redirected", "Mr R", None).await.is_err(), "no service");
    assert!(
        t("redirected", "Mr R", Some("  ".into())).await.is_err(),
        "blank service"
    );
    assert!(
        t("redirected", "Mr R", Some(w.service.clone()))
            .await
            .is_err(),
        "the same service"
    );
    assert!(
        t("returned_with_advice", "Mr R", Some(w.other.clone()))
            .await
            .is_err(),
        "a service on a return"
    );
    let r = w.s.referrals.get(id).await.unwrap();
    assert_eq!(r.status, "received");
    assert!(r.triaged_by.is_none());
    assert_eq!(w.audit_rows(id).await, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn only_a_received_referral_can_be_triaged() {
    let w = world().await;
    let waitlisted = w.received(Uuid::new_v4()).await;
    w.s.referrals
        .waitlist(waitlisted, Priority::Routine, &sys())
        .await
        .unwrap();
    let closed = w.received(Uuid::new_v4()).await;
    w.s.referrals.close(closed, None, &sys()).await.unwrap();
    for id in [waitlisted, closed] {
        let e =
            w.s.referrals
                .triage(id, "returned_with_advice", "Mr R", None, &sys())
                .await
                .unwrap_err()
                .to_string();
        assert!(
            e.contains("only a referral that has just been received"),
            "{e}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoint_triages_once_and_refuses_the_repeat() {
    let w = world().await;
    let id = w.received(Uuid::new_v4()).await;
    let app = patient_administration_system::api::rest::router(w.s.clone());
    let post = |body: serde_json::Value| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/referrals/{id}/triage"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = r.status();
            let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            )
        }
    };
    let body = serde_json::json!({ "outcome": "returned_with_advice", "triaged_by": "Mr R" });
    let (status, v) = post(body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["status"], "returned_with_advice");
    let (status, v) = post(body).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
}
