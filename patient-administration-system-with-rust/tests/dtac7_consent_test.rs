//! DTAC-7: a use that needs a patient's consent answers 403 without it. Needs PostgreSQL.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::consent::ConsentRepository;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::communication::{DeliveryChannel, LetterTemplate};
use patient_administration_system::models::consent::{Consent, ConsentType};
use patient_administration_system::privacy::consent::{has_consent, require_consent};
use patient_administration_system::upstream::{Person, StubPersonClient};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

async fn world() -> (
    patient_administration_system::api::rest::AppState,
    Uuid,
    Uuid,
) {
    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub);
    let mut t = LetterTemplate::new(
        format!("c-{}", Uuid::new_v4()),
        "Hi".into(),
        "Hello.".into(),
    );
    t.channels = vec![DeliveryChannel::Print];
    let template = LetterRepository::create_template(&s.db, &t)
        .await
        .unwrap()
        .id;
    (s, person, template)
}

async fn grant(
    s: &patient_administration_system::api::rest::AppState,
    person: Uuid,
    kind: ConsentType,
) -> Consent {
    let c = Consent::new(person, kind, Utc::now().date_naive() - Duration::days(1));
    ConsentRepository::create(&s.db, &c).await.unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn consent_is_required_active_unexpired_unrevoked_and_of_the_right_kind() {
    let (s, p, _) = world().await;
    assert!(
        require_consent(&s.db, p, ConsentType::Marketing)
            .await
            .is_err()
    );
    // A consent of another kind does not count.
    grant(&s, p, ConsentType::Research).await;
    assert!(!has_consent(&s.db, p, ConsentType::Marketing).await.unwrap());
    // Expired.
    let mut old = Consent::new(
        p,
        ConsentType::Marketing,
        Utc::now().date_naive() - Duration::days(30),
    );
    old.expiry_date = Some(Utc::now().date_naive() - Duration::days(1));
    ConsentRepository::create(&s.db, &old).await.unwrap();
    assert!(!has_consent(&s.db, p, ConsentType::Marketing).await.unwrap());
    // Active.
    let c = grant(&s, p, ConsentType::Marketing).await;
    require_consent(&s.db, p, ConsentType::Marketing)
        .await
        .unwrap();
    // Revoked.
    ConsentRepository::revoke(&s.db, c.id).await.unwrap();
    let err = require_consent(&s.db, p, ConsentType::Marketing)
        .await
        .unwrap_err();
    assert!(
        matches!(err, patient_administration_system::Error::Forbidden(_)),
        "{err}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_marketing_message_is_refused_without_consent_and_a_service_message_needs_none() {
    let (s, p, template) = world().await;
    let marketing = serde_json::json!({ "purpose": "marketing" });
    let err = s
        .communication
        .generate_letter(
            template,
            p,
            None,
            DeliveryChannel::Print,
            marketing.clone(),
            &sys(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, patient_administration_system::Error::Forbidden(_)),
        "{err}"
    );
    // A service message (no purpose, or "service") is not held to it.
    s.communication
        .generate_letter(
            template,
            p,
            None,
            DeliveryChannel::Print,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap();
    s.communication
        .generate_letter(
            template,
            p,
            None,
            DeliveryChannel::Print,
            serde_json::json!({ "purpose": "service" }),
            &sys(),
        )
        .await
        .unwrap();
    // With consent, the marketing message goes.
    grant(&s, p, ConsentType::Marketing).await;
    s.communication
        .generate_letter(template, p, None, DeliveryChannel::Print, marketing, &sys())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoint_answers_403_for_marketing_without_consent() {
    let (s, p, template) = world().await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let post = |extra: serde_json::Value| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/letters/generate")
                        .header("content-type", "application/json")
                        .body(Body::from(serde_json::json!({ "template_id": template, "person_id": p, "channel": "print", "extra": extra }).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
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
    let (st, v) = post(serde_json::json!({ "purpose": "marketing" })).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "FORBIDDEN");
    let (st, _) = post(serde_json::json!({})).await;
    assert_eq!(st, StatusCode::OK);
    grant(&s, p, ConsentType::Marketing).await;
    let (st, _) = post(serde_json::json!({ "purpose": "marketing" })).await;
    assert_eq!(st, StatusCode::OK);
}
