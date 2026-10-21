//! NOT-7: the postal letter print queue. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Letters left pending by earlier runs are closed out first, so this test's batch holds only its own.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::communication::{DeliveryChannel, LetterTemplate};
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    template: Uuid,
    /// (person, has an address)
    people: Vec<(Uuid, bool)>,
}

async fn world(with_address: &[bool]) -> W {
    let stub = Arc::new(StubPersonClient::new());
    let mut people = Vec::new();
    for has in with_address {
        let id = Uuid::new_v4();
        let mut data = serde_json::Map::new();
        data.insert(
            "name".into(),
            serde_json::json!({ "given": ["Ada", "Augusta"], "family": "Lovelace" }),
        );
        if *has {
            data.insert("addresses".into(), serde_json::json!([{ "line": ["12 Mill Lane"], "city": "Cardiff", "postal_code": "CF10 1AA" }]));
        }
        stub.insert(Person { id, data });
        people.push((id, *has));
    }
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub);
    // Close out print letters earlier runs left pending.
    s.db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        "UPDATE generated_letters SET status = 'failed' WHERE channel = 'print' AND status = 'pending'".to_string(),
    ))
    .await
    .unwrap();
    let mut t = LetterTemplate::new(
        format!("print-{}", Uuid::new_v4()),
        "=Your appointment".into(),
        "Dear {{ patient.name.family }}, see you soon.".into(),
    );
    t.channels = vec![DeliveryChannel::Print];
    let template = LetterRepository::create_template(&s.db, &t)
        .await
        .unwrap()
        .id;
    W {
        s,
        template,
        people,
    }
}

impl W {
    async fn letter(&self, person: Uuid) -> Uuid {
        self.s
            .communication
            .generate_letter(
                self.template,
                person,
                None,
                DeliveryChannel::Print,
                serde_json::json!({}),
                &sys(),
            )
            .await
            .unwrap()
            .id
    }
    async fn status(&self, letter: Uuid) -> String {
        self.s
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT status FROM generated_letters WHERE id = $1",
                [letter.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "status")
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn export_then_mark_sent_flips_exactly_the_exported_letters() {
    let w = world(&[true, true, true, false]).await;
    let mut ids = Vec::new();
    for (p, _) in &w.people {
        ids.push(w.letter(*p).await);
    }
    let (waiting, items) = w.s.printing.queue(100).await.unwrap();
    assert_eq!(waiting, 4);
    assert_eq!(items.len(), 4);

    let batch =
        w.s.printing
            .export_batch(100, "Clerk A", &sys())
            .await
            .unwrap();
    assert_eq!(batch.rows.len(), 3);
    assert_eq!(
        batch.skipped_no_address, 1,
        "no address on record: left in the queue, not lost"
    );
    assert_eq!(batch.rows[0].name, "Ada Augusta Lovelace");
    assert_eq!(batch.rows[0].address, "12 Mill Lane, Cardiff, CF10 1AA");
    // Exporting posts nothing: the letters stay pending until a person says they went.
    for id in &ids {
        assert_eq!(w.status(*id).await, "pending");
    }
    assert_eq!(
        w.s.printing.queue(100).await.unwrap().0,
        1,
        "exported letters have left the queue"
    );

    let n =
        w.s.printing
            .mark_sent(batch.batch_id, "Clerk A", &sys())
            .await
            .unwrap();
    assert_eq!(n, 3);
    for (id, (_, has)) in ids.iter().zip(&w.people) {
        assert_eq!(
            w.status(*id).await,
            if *has { "sent" } else { "pending" },
            "exactly the exported ones"
        );
    }
    // Once, and it must be a real batch and a named person.
    assert!(
        w.s.printing
            .mark_sent(batch.batch_id, "Clerk A", &sys())
            .await
            .is_err()
    );
    assert!(
        w.s.printing
            .mark_sent(Uuid::new_v4(), "Clerk A", &sys())
            .await
            .is_err()
    );
    assert!(w.s.printing.export_batch(10, " ", &sys()).await.is_err());
    assert_eq!(
        common::audit_count(&w.s.db, batch.batch_id, "sent").await,
        1
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_letter_is_in_at_most_one_batch_and_an_empty_queue_is_said_plainly() {
    let w = world(&[true, true, true, true, true, true]).await;
    let mut ids = Vec::new();
    for (p, _) in &w.people {
        ids.push(w.letter(*p).await);
    }
    // Two exports at once take different letters.
    let ctx = sys();
    let (a, b) = tokio::join!(
        w.s.printing.export_batch(3, "A", &ctx),
        w.s.printing.export_batch(3, "B", &ctx)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    let mut all: Vec<Uuid> = a.rows.iter().chain(&b.rows).map(|r| r.reference).collect();
    assert_eq!(all.len(), 6);
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 6, "no letter in two batches");
    // Nothing is left to export.
    let err =
        w.s.printing
            .export_batch(10, "A", &sys())
            .await
            .unwrap_err();
    assert!(err.to_string().contains("no queued letters"), "{err}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_address_is_not_kept_and_the_csv_defuses_formulas() {
    let w = world(&[true]).await;
    w.letter(w.people[0].0).await;
    let batch =
        w.s.printing
            .export_batch(10, "Clerk A", &sys())
            .await
            .unwrap();
    let csv = batch.to_csv();
    assert!(csv.starts_with("reference,name,address,subject,body\n"));
    assert!(csv.contains("\"12 Mill Lane, Cardiff, CF10 1AA\""));
    assert!(
        csv.contains("'=Your appointment"),
        "a subject starting with = cannot run as a formula: {csv}"
    );
    let kept =
        w.s.db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM audit_log WHERE new_value::text LIKE '%Mill Lane%'"
                    .to_string(),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "n")
            .unwrap();
    assert_eq!(
        kept, 0,
        "the address is in the export and nowhere in the PAS"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_queue_export_and_mark_sent() {
    let w = world(&[true, true]).await;
    let l1 = w.letter(w.people[0].0).await;
    w.letter(w.people[1].0).await;
    let app = patient_administration_system::api::rest::router(w.s.clone());
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
            let hdr = r
                .headers()
                .get("x-batch-id")
                .map(|v| v.to_str().unwrap().to_string());
            let text = String::from_utf8(to_bytes(r.into_body(), 1 << 20).await.unwrap().to_vec())
                .unwrap();
            (st, hdr, text)
        }
    };
    let (st, _, body) = call("GET", "/api/print-queue".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(body.contains("\"waiting\":2"), "{body}");
    let (st, _, _) = call(
        "POST",
        "/api/print-batches".into(),
        Some(serde_json::json!({ "created_by": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, hdr, csv) = call(
        "POST",
        "/api/print-batches".into(),
        Some(serde_json::json!({ "created_by": "Clerk A", "format": "csv" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{csv}");
    let batch = hdr.expect("the batch id is in a header for the CSV");
    assert_eq!(csv.lines().count(), 3, "header and two letters");
    let (st, _, body) = call(
        "POST",
        format!("/api/print-batches/{batch}/sent"),
        Some(serde_json::json!({ "sent_by": "Clerk A" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body.contains("\"letters_sent\":2"));
    let (st, _, _) = call(
        "POST",
        format!("/api/print-batches/{batch}/sent"),
        Some(serde_json::json!({ "sent_by": "Clerk A" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, _, _) = call(
        "POST",
        format!("/api/print-batches/{}/sent", Uuid::new_v4()),
        Some(serde_json::json!({ "sent_by": "A" })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(w.status(l1).await, "sent");
}
