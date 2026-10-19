//! CORE-5: letter preview. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::communication::{DeliveryChannel, LetterTemplate};
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn setup(
    body: &str,
    required: &[&str],
) -> (
    patient_administration_system::api::rest::AppState,
    Uuid,
    Uuid,
) {
    let person = Uuid::new_v4();
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        serde_json::json!({ "given": "Ada", "family": "Lovelace" }),
    );
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub);
    let mut t = LetterTemplate::new(
        format!("preview-{}", Uuid::new_v4()),
        "Hello {{ patient.name.given }}".into(),
        body.into(),
    );
    t.channels = vec![DeliveryChannel::Sms];
    t.required_variables = required.iter().map(|r| r.to_string()).collect();
    let t = LetterRepository::create_template(&s.db, &t).await.unwrap();
    (s, person, t.id)
}

async fn counts(
    s: &patient_administration_system::api::rest::AppState,
    person: Uuid,
) -> (i64, i64) {
    let n = |sql: &'static str| {
        let db = s.db.clone();
        async move {
            db.query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                [person.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "n")
            .unwrap()
        }
    };
    (
        n("SELECT count(*) AS n FROM generated_letters WHERE person_id = $1").await,
        n("SELECT count(*) AS n FROM audit_log WHERE entity_type = 'generated_letter' AND new_value::text LIKE '%' || $1::text || '%'").await,
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_preview_renders_what_would_be_sent_and_writes_nothing() {
    let (s, person, template) = setup(
        "Dear {{ patient.name.family }}, see you on {{ when }}.",
        &["when"],
    )
    .await;
    let before = counts(&s, person).await;
    let (subject, body, _) = s
        .communication
        .preview_letter(
            template,
            person,
            serde_json::json!({ "when": "Tue 3 Nov" }),
            &Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(subject, "Hello Ada");
    assert_eq!(body, "Dear Lovelace, see you on Tue 3 Nov.");
    assert_eq!(
        counts(&s, person).await,
        before,
        "no letter and no audit row was written"
    );

    // The preview is the same text a real send stores.
    let real = s
        .communication
        .generate_letter(
            template,
            person,
            None,
            DeliveryChannel::Print,
            serde_json::json!({ "when": "Tue 3 Nov" }),
            &Default::default(),
        )
        .await
        .unwrap();
    assert_eq!((real.rendered_subject, real.rendered_body), (subject, body));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_missing_variable_is_reported_by_name_and_nothing_is_written() {
    let (s, person, template) = setup("On {{ when }}.", &["when"]).await;
    let err = s
        .communication
        .preview_letter(template, person, serde_json::json!({}), &Default::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("when"), "{err}");
    assert_eq!(counts(&s, person).await.0, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoint_previews_reports_and_404s() {
    let (s, person, template) =
        setup("Dear {{ patient.name.family }}, {{ when }}.", &["when"]).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let post = |body: serde_json::Value| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/letters/preview")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
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
    let (st, v) = post(serde_json::json!({ "template_id": template, "person_id": person, "extra": { "when": "Tuesday" } })).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["rendered_body"], "Dear Lovelace, Tuesday.");
    assert!(v["data"]["sms"].is_object(), "the SMS length is shown too");
    let (st, v) = post(serde_json::json!({ "template_id": template, "person_id": person })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(v["error"]["message"].as_str().unwrap().contains("when"));
    let (st, _) =
        post(serde_json::json!({ "template_id": Uuid::new_v4(), "person_id": person })).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(counts(&s, person).await.0, 0);
}
