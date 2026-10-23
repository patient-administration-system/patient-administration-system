//! NEW-3: patient-initiated follow-up. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::pifu::{Decision, NewEpisode, PifuService};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

fn new<'a>(service: &'a str, days: i64) -> NewEpisode<'a> {
    NewEpisode {
        service_type: service,
        conditions: "Ring us if the pain returns or you cannot walk to the shops.",
        expires_at: Utc::now() + Duration::days(days),
        opened_by: "Dr Evans",
        urgent_override_by: None,
    }
}

async fn setup() -> (
    patient_administration_system::api::rest::AppState,
    PifuService,
    UserContext,
) {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = PifuService::new(state.db.clone());
    (state, svc, UserContext::system())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_request_inside_the_window_is_a_booking_task_and_is_handled_once() {
    let (_s, svc, ctx) = setup().await;
    let person = Uuid::new_v4();
    let now = Utc::now();
    let ep = svc
        .open(person, &new("new3-a", 180), now, &ctx)
        .await
        .unwrap();

    let r = svc.request(ep.id, "text", now, &ctx).await.unwrap();
    assert_eq!(r.route, "booking");
    // A second request while one is open is the same request.
    let again = svc.request(ep.id, "portal", now, &ctx).await.unwrap();
    assert_eq!(again.id, r.id);

    let queue = svc.open_requests("booking", 501).await.unwrap();
    assert!(queue.iter().any(|q| q.id == r.id));
    svc.handle(r.id, "Booker", None, now, &ctx).await.unwrap();
    assert!(
        svc.handle(r.id, "Booker", None, now, &ctx).await.is_err(),
        "handled once only"
    );
    let queue = svc.open_requests("booking", 501).await.unwrap();
    assert!(!queue.iter().any(|q| q.id == r.id));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn expiry_creates_a_review_task_and_never_discharges_by_itself() {
    let (_s, svc, ctx) = setup().await;
    let person = Uuid::new_v4();
    let now = Utc::now();
    let ep = svc
        .open(person, &new("new3-b", 30), now, &ctx)
        .await
        .unwrap();

    let later = now + Duration::days(31);
    let due = svc.due_reviews(later, 501).await.unwrap();
    assert!(due.iter().any(|e| e.id == ep.id), "the review task exists");
    assert!(
        !svc.due_reviews(now, 501)
            .await
            .unwrap()
            .iter()
            .any(|e| e.id == ep.id),
        "not before the window ends"
    );
    // The episode is still active: expiry alone changed nothing.
    let status = status_of(&_s.db, ep.id).await;
    assert_eq!(status, "active");

    // Not before the end, not for a blank name; extend then discharge by a person.
    assert!(
        svc.review(ep.id, Decision::Discharge, "Dr Evans", now, &ctx)
            .await
            .is_err()
    );
    assert!(
        svc.review(ep.id, Decision::Discharge, " ", later, &ctx)
            .await
            .is_err()
    );
    svc.review(
        ep.id,
        Decision::Extend(later + Duration::days(90)),
        "Dr Evans",
        later,
        &ctx,
    )
    .await
    .unwrap();
    assert!(
        !svc.due_reviews(later, 501)
            .await
            .unwrap()
            .iter()
            .any(|e| e.id == ep.id)
    );
    let much_later = later + Duration::days(91);
    svc.review(ep.id, Decision::Discharge, "Dr Evans", much_later, &ctx)
        .await
        .unwrap();
    assert!(
        svc.review(ep.id, Decision::Discharge, "Dr Evans", much_later, &ctx)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_request_after_the_window_goes_to_a_person_and_blocks_the_review_until_handled() {
    let (_s, svc, ctx) = setup().await;
    let person = Uuid::new_v4();
    let now = Utc::now();
    let ep = svc
        .open(person, &new("new3-c", 10), now, &ctx)
        .await
        .unwrap();
    let later = now + Duration::days(11);

    let r = svc.request(ep.id, "phone", later, &ctx).await.unwrap();
    assert_eq!(
        r.route, "person",
        "after expiry a person decides, nothing is booked"
    );
    assert!(
        svc.open_requests("booking", 501)
            .await
            .unwrap()
            .iter()
            .all(|q| q.id != r.id),
        "not on the booking queue"
    );
    assert!(
        svc.open_requests("person", 501)
            .await
            .unwrap()
            .iter()
            .any(|q| q.id == r.id)
    );
    assert!(
        svc.review(ep.id, Decision::Discharge, "Dr Evans", later, &ctx)
            .await
            .is_err(),
        "an open request is dealt with before a discharge"
    );
    svc.handle(r.id, "Dr Evans", None, later, &ctx)
        .await
        .unwrap();
    svc.review(ep.id, Decision::Discharge, "Dr Evans", later, &ctx)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_urgent_service_needs_a_named_override_and_bad_input_is_refused() {
    let (state, svc, ctx) = setup().await;
    let now = Utc::now();
    state
        .db
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            "INSERT INTO follow_up_policies (service_type, approved_by) VALUES ('new3-urgent', 'Dr Approver') \
             ON CONFLICT DO NOTHING",
        ))
        .await
        .unwrap();
    let p = Uuid::new_v4();
    assert!(
        svc.open(p, &new("new3-urgent", 90), now, &ctx)
            .await
            .is_err()
    );
    let mut with = new("new3-urgent", 90);
    with.urgent_override_by = Some("Dr Override");
    svc.open(p, &with, now, &ctx).await.unwrap();
    assert!(
        svc.open(p, &with, now, &ctx).await.is_err(),
        "one live episode per patient and service"
    );

    let q = Uuid::new_v4();
    assert!(
        svc.open(q, &new("new3-d", -1), now, &ctx).await.is_err(),
        "ends in the past"
    );
    assert!(
        svc.open(q, &new("new3-d", 800), now, &ctx).await.is_err(),
        "over two years"
    );
    let mut short = new("new3-d", 30);
    short.conditions = "too short";
    assert!(svc.open(q, &short, now, &ctx).await.is_err());
    let mut anon = new("new3-d", 30);
    anon.opened_by = "";
    assert!(svc.open(q, &anon, now, &ctx).await.is_err());
    let ep = svc.open(q, &new("new3-d", 30), now, &ctx).await.unwrap();
    assert!(
        svc.request(ep.id, "carrier-pigeon", now, &ctx)
            .await
            .is_err()
    );
    assert!(svc.uptake(10).await.unwrap().cells.len() == 5);
}

async fn status_of(db: &sea_orm::DatabaseConnection, id: Uuid) -> String {
    db.query_one_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT status FROM pifu_episodes WHERE id = $1",
        [id.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get("", "status")
    .unwrap()
}
