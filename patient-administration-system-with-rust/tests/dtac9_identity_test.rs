//! DTAC-9: an account is served by the patient portal only after staff record that they proofed it as that
//! patient. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::db::repositories::audit::UserContext;
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_account_is_proofed_only_as_the_patient_staff_recorded_and_only_until_revoked() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let (sub, person, other) = (
        format!("kc-{}", Uuid::new_v4()),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    assert!(
        !s.identity.check(&sub, person).await.unwrap(),
        "an unproofed account reaches nothing"
    );

    s.identity
        .record(&sub, person, "in_person", "Clerk A", &sys())
        .await
        .unwrap();
    assert!(s.identity.check(&sub, person).await.unwrap());
    assert!(
        !s.identity.check(&sub, other).await.unwrap(),
        "proofed as one patient, not another"
    );
    assert!(
        !s.identity.check("someone-else", person).await.unwrap(),
        "and not for another account"
    );
    assert_eq!(
        common::audit_count(&s.db, person, "identity_proofed").await,
        1,
        "audited against the patient"
    );
    let p = s.identity.get(&sub).await.unwrap().unwrap();
    assert_eq!(
        (p.method.as_str(), p.proofed_by.as_str()),
        ("in_person", "Clerk A")
    );

    // Revoking cuts it off at once and keeps the history.
    s.identity.revoke(&sub, "Clerk B", &sys()).await.unwrap();
    assert!(!s.identity.check(&sub, person).await.unwrap());
    assert!(
        s.identity.revoke(&sub, "Clerk B", &sys()).await.is_err(),
        "nothing live to revoke"
    );
    assert_eq!(
        common::audit_count(&s.db, person, "identity_proofing_revoked").await,
        1
    );
    let rows = {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        s.db.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM identity_proofings WHERE subject = $1",
            [sub.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "n")
        .unwrap()
    };
    assert_eq!(rows, 1, "the row is kept, marked revoked");

    // Proofed again, as a different patient: the old link is superseded, not left live alongside.
    s.identity
        .record(&sub, other, "video_id_check", "Clerk C", &sys())
        .await
        .unwrap();
    assert!(s.identity.check(&sub, other).await.unwrap());
    assert!(!s.identity.check(&sub, person).await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn proofing_needs_a_known_method_a_named_person_and_a_sane_subject() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let (sub, p) = (format!("kc-{}", Uuid::new_v4()), Uuid::new_v4());
    assert!(
        s.identity
            .record(&sub, p, "vibes", "A", &sys())
            .await
            .is_err()
    );
    assert!(
        s.identity
            .record(&sub, p, "in_person", "  ", &sys())
            .await
            .is_err()
    );
    assert!(
        s.identity
            .record("", p, "in_person", "A", &sys())
            .await
            .is_err()
    );
    assert!(
        s.identity
            .record(&"x".repeat(201), p, "in_person", "A", &sys())
            .await
            .is_err()
    );
    assert!(s.identity.revoke(&sub, " ", &sys()).await.is_err());
    assert!(
        !s.identity.check(&sub, p).await.unwrap(),
        "nothing was recorded"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_record_check_and_revoke() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let call = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let mut b = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    b = b.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(b.body(body).unwrap()).await.unwrap();
            let st = r.status();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(
                    &to_bytes(r.into_body(), 1 << 20).await.unwrap(),
                )
                .unwrap_or_default(),
            )
        }
    };
    let (sub, person) = (format!("kc-{}", Uuid::new_v4()), Uuid::new_v4());
    let (st, v) = call(
        "GET",
        format!("/api/identity-proofings/{sub}/check?person_id={person}"),
        None,
    )
    .await;
    assert_eq!(
        (st, v["data"]["proofed"].as_bool()),
        (StatusCode::OK, Some(false))
    );
    let (st, _) = call(
        "PUT",
        format!("/api/identity-proofings/{sub}"),
        Some(serde_json::json!({ "person_id": person, "method": "in_person", "proofed_by": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call("PUT", format!("/api/identity-proofings/{sub}"), Some(serde_json::json!({ "person_id": person, "method": "in_person", "proofed_by": "Clerk A" }))).await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = call(
        "GET",
        format!("/api/identity-proofings/{sub}/check?person_id={person}"),
        None,
    )
    .await;
    assert_eq!(v["data"]["proofed"], true);
    let (st, v) = call("GET", format!("/api/identity-proofings/{sub}"), None).await;
    assert_eq!(
        (st, v["data"]["method"].as_str()),
        (StatusCode::OK, Some("in_person"))
    );
    let (st, _) = call(
        "DELETE",
        format!("/api/identity-proofings/{sub}?revoked_by=Clerk%20B"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = call(
        "GET",
        format!("/api/identity-proofings/{sub}/check?person_id={person}"),
        None,
    )
    .await;
    assert_eq!(v["data"]["proofed"], false);
    let (st, _) = call(
        "DELETE",
        format!("/api/identity-proofings/{sub}?revoked_by=Clerk%20B"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}
