//! TSC-8: a patient we tried every way and heard nothing back from becomes work for a person to try by phone.
//! Each of the three facts (the ladder's call unanswered, a message bounced, a validation letter past its
//! reply-by date) makes one item, and a later answer closes it. Needs PostgreSQL:
//! `cargo test -- --ignored --test-threads=1`.

mod common;

use jiff::Timestamp;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Pool, Stmt};
use patient_administration_system::models::communication::LetterTemplate;
use patient_administration_system::work_queue::{PhoneOutcome, WorkKind, WorkQueueService};
use uuid::Uuid;

async fn db() -> Pool {
    common::build_state(&common::database_url().unwrap())
        .await
        .db
}

fn ctx() -> UserContext {
    UserContext {
        user_id: Some("Sam".into()),
        user_ip: None,
        user_agent: None,
    }
}

async fn run(db: &Pool, sql: &str, params: Vec<patient_administration_system::db::sql::Param>) {
    db.run(Stmt::new(sql, params)).await.unwrap();
}

/// The unreached item for `source`: its id and status.
async fn item(db: &Pool, source: Uuid) -> Option<(Uuid, String)> {
    db.one(Stmt::new(
        "SELECT id, status FROM work_items WHERE kind = 'unreached' AND source_id = $1",
        [source.into()],
    ))
    .await
    .unwrap()
    .map(|r| (r.col("id").unwrap(), r.col("status").unwrap()))
}

async fn unanswered_call(db: &Pool, person: Uuid, hours_ago: i64) -> Uuid {
    let id = Uuid::new_v4();
    run(
        db,
        "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at, handled_at, handled_by, call_outcome) \
         VALUES ($1, $2, $3, 'silent', now(), now() - make_interval(hours => $4::int), 'Sam', 'no_answer')",
        vec![id.into(), Uuid::new_v4().into(), person.into(), hours_ago.into()],
    )
    .await;
    id
}

async fn text_from(db: &Pool, person: Uuid, hours_ago: i64) {
    run(
        db,
        "INSERT INTO inbound_texts (id, message_id, from_e164, body, received_at, person_id, intent, outcome) \
         VALUES ($1, $2, '+447700900123', 'Y', now() - make_interval(hours => $3::int), $4, 'confirm', 'applied')",
        vec![Uuid::new_v4().into(), format!("m-{}", Uuid::new_v4()).into(), hours_ago.into(), person.into()],
    )
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_unanswered_ladder_call_makes_one_item_and_a_later_text_closes_it() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let person = Uuid::new_v4();
    // A text from before the call answers nothing the call asked.
    text_from(&db, person, 5).await;
    let call = unanswered_call(&db, person, 2).await;
    svc.sync(Timestamp::now()).await.unwrap();
    svc.sync(Timestamp::now()).await.unwrap();
    let (id, status) = item(&db, call).await.expect("an item");
    assert_eq!(status, "open");
    let n = db
        .count(Stmt::new(
            "SELECT count(*) AS n FROM work_items WHERE kind = 'unreached' AND person_id = $1",
            [person.into()],
        ))
        .await
        .unwrap();
    assert_eq!(n, 1, "one item, however often the queue syncs");
    let got = svc.get(id).await.unwrap();
    assert_eq!(
        (got.queue.as_str(), got.subject_kind.as_deref()),
        ("booking", Some("appointment"))
    );

    text_from(&db, person, 0).await;
    svc.sync(Timestamp::now()).await.unwrap();
    assert_eq!(
        item(&db, call).await.unwrap().1,
        "closed",
        "the patient answered"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_bounced_message_makes_one_item_and_a_reply_closes_it() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let person = Uuid::new_v4();
    let t = LetterRepository::create_template(
        &db,
        &LetterTemplate::new(format!("tsc8-{}", Uuid::new_v4()), "S".into(), "B".into()),
    )
    .await
    .unwrap();
    let letter = Uuid::new_v4();
    run(
        &db,
        "INSERT INTO generated_letters (id, template_id, person_id, rendered_subject, rendered_body, channel, status, created_at, updated_at) \
         VALUES ($1, $2, $3, 'S', 'B', 'email', 'sent', now() - interval '1 day', now())",
        vec![letter.into(), t.id.into(), person.into()],
    )
    .await;
    let bounce = Uuid::new_v4();
    let receipt = |id: Uuid, event: &'static str, hours_ago: i64| {
        let db = db.clone();
        async move {
            run(
                &db,
                "INSERT INTO message_receipts (id, letter_id, event, carrier, receipt_id, occurred_at) \
                 VALUES ($1, $2, $3, 'test', $4, now() - make_interval(hours => $5::int))",
                vec![id.into(), letter.into(), event.into(), Uuid::new_v4().to_string().into(), hours_ago.into()],
            )
            .await;
        }
    };
    receipt(bounce, "bounced", 3).await;
    svc.sync(Timestamp::now()).await.unwrap();
    let (id, status) = item(&db, bounce).await.expect("an item");
    assert_eq!(status, "open");
    assert_eq!(
        svc.get(id).await.unwrap().subject_kind.as_deref(),
        Some("profile")
    );
    receipt(Uuid::new_v4(), "replied", 0).await;
    svc.sync(Timestamp::now()).await.unwrap();
    assert_eq!(item(&db, bounce).await.unwrap().1, "closed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_unanswered_validation_letter_makes_one_item_and_an_answer_closes_it() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let (person, pathway, campaign, validation) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    run(
        &db,
        "INSERT INTO rtt_pathways (id, person_id, target_service, breach_weeks, status, started_at, created_at, updated_at) \
         VALUES ($1, $2, 'cardiology', 18, 'active', now() - interval '20 weeks', now(), now())",
        vec![pathway.into(), person.into()],
    )
    .await;
    run(
        &db,
        "INSERT INTO validation_campaigns (id, name, min_weeks, started_by, reply_by, final_wait_days) \
         VALUES ($1, 'tsc8', 12, 'Validator', current_date - 30, 7)",
        vec![campaign.into()],
    )
    .await;
    run(
        &db,
        "INSERT INTO validation_items (id, campaign_id, pathway_id, person_id, target_service, state, letter_sent_by, letter_sent_at, final_by) \
         VALUES ($1, $2, $3, $4, 'cardiology', 'letter_sent', 'Validator', now() - interval '10 days', current_date - 1)",
        vec![validation.into(), campaign.into(), pathway.into(), person.into()],
    )
    .await;
    svc.sync(Timestamp::now()).await.unwrap();
    let (id, status) = item(&db, validation).await.expect("an item");
    assert_eq!(status, "open");
    assert_eq!(
        svc.get(id).await.unwrap().subject_kind.as_deref(),
        Some("pathway")
    );
    run(
        &db,
        "UPDATE validation_items SET state = 'answered', answer = 'still_needed', answered_at = now() WHERE id = $1",
        vec![validation.into()],
    )
    .await;
    svc.sync(Timestamp::now()).await.unwrap();
    assert_eq!(item(&db, validation).await.unwrap().1, "closed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_phone_outcome_closes_the_item_for_good_and_only_an_unreached_item_takes_one() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let person = Uuid::new_v4();
    let call = unanswered_call(&db, person, 1).await;
    svc.sync(Timestamp::now()).await.unwrap();
    let (id, _) = item(&db, call).await.unwrap();
    assert!(
        svc.record_phone_outcome(id, PhoneOutcome::Reached, " ", &ctx())
            .await
            .is_err(),
        "a named person"
    );
    let closed = svc
        .record_phone_outcome(id, PhoneOutcome::WrongNumber, "Sam", &ctx())
        .await
        .unwrap();
    assert_eq!(closed.status.as_str(), "closed");
    assert_eq!(common::audit_count(&db, id, "phone_outcome").await, 1);
    svc.sync(Timestamp::now()).await.unwrap();
    assert_eq!(
        item(&db, call).await.unwrap().1,
        "closed",
        "a sync does not open it again"
    );
    assert!(
        svc.record_phone_outcome(id, PhoneOutcome::Reached, "Sam", &ctx())
            .await
            .is_err(),
        "closed once"
    );

    // An item of another kind closes in its own list.
    let silent = Uuid::new_v4();
    run(
        &db,
        "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) VALUES ($1, $2, $3, 'silent', now())",
        vec![silent.into(), Uuid::new_v4().into(), person.into()],
    )
    .await;
    svc.sync(Timestamp::now()).await.unwrap();
    let other: Uuid = db
        .one(Stmt::new(
            "SELECT id FROM work_items WHERE kind = $1 AND source_id = $2",
            [WorkKind::Call.as_str().into(), silent.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .col("id")
        .unwrap();
    assert!(matches!(
        svc.record_phone_outcome(other, PhoneOutcome::Reached, "Sam", &ctx())
            .await,
        Err(patient_administration_system::Error::Conflict(_))
    ));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_returned_paper_letter_is_recorded_once_and_makes_the_patient_unreached() {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    let state = common::build_state(&common::database_url().unwrap()).await;
    let db = state.db.clone();
    let person = Uuid::new_v4();
    let t = LetterRepository::create_template(
        &db,
        &LetterTemplate::new(
            format!("tsc8-post-{}", Uuid::new_v4()),
            "S".into(),
            "B".into(),
        ),
    )
    .await
    .unwrap();
    let letter = |channel: &'static str| {
        let db = db.clone();
        async move {
            let id = Uuid::new_v4();
            run(
                &db,
                "INSERT INTO generated_letters (id, template_id, person_id, rendered_subject, rendered_body, channel, status, created_at, updated_at) \
                 VALUES ($1, $2, $3, 'S', 'B', $4, 'sent', now() - interval '3 days', now())",
                vec![id.into(), t.id.into(), person.into(), channel.into()],
            )
            .await;
            id
        }
    };
    let (paper, email) = (letter("print").await, letter("email").await);
    let app = patient_administration_system::api::rest::router(state);
    let post = |id: Uuid, body: &'static str| {
        app.clone().oneshot(
            Request::post(format!("/api/letters/{id}/returned"))
                .header("content-type", "application/json")
                .header("x-user-id", "Post room")
                .body(Body::from(body))
                .unwrap(),
        )
    };
    let r = post(paper, r#"{"reason":"gone_away"}"#).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap();
    assert_eq!(v["data"]["duplicate"], false, "{v}");
    let r = post(paper, "{}").await.unwrap();
    let v: serde_json::Value =
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap();
    assert_eq!(v["data"]["duplicate"], true, "once per letter: {v}");
    assert_eq!(
        post(email, "{}").await.unwrap().status(),
        StatusCode::CONFLICT,
        "only paper comes back by post"
    );
    assert_eq!(
        post(Uuid::new_v4(), "{}").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );

    let status: String = db
        .one(Stmt::new(
            "SELECT status FROM generated_letters WHERE id = $1",
            [paper.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .col("status")
        .unwrap();
    assert_eq!(status, "failed");
    let receipt: Uuid = db
        .one(Stmt::new(
            "SELECT id FROM message_receipts WHERE letter_id = $1 AND carrier = 'post' AND event = 'bounced' AND detail = 'gone_away'",
            [paper.into()],
        ))
        .await
        .unwrap()
        .expect("the post's bounce")
        .col("id")
        .unwrap();
    WorkQueueService::new(db.clone())
        .sync(Timestamp::now())
        .await
        .unwrap();
    assert_eq!(
        item(&db, receipt).await.unwrap().1,
        "open",
        "a person is asked to phone"
    );
}
