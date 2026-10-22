//! NEW-18: group consultations. A slot declares its capacity before booking; concurrent bookings never
//! exceed it, and a one-to-one slot still holds one appointment. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::group_sessions::GroupSessionService;
use patient_administration_system::models::appointment::CancellationReason;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

async fn live(db: &sea_orm::DatabaseConnection, slot: Uuid) -> i64 {
    db.query_one_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT count(*)::bigint AS n FROM appointments WHERE slot_id = $1 AND deleted_at IS NULL \
         AND status IN ('booked','arrived','fulfilled')",
        [slot.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get("", "n")
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_crowd_booking_a_group_session_at_once_fills_it_exactly_and_no_further() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let groups = GroupSessionService::new(s.db.clone());
    for round in 0..3 {
        let schedule = common::new_schedule(&s.db).await.id;
        let slot = common::free_slot(&s.db, schedule, 420 + round).await;
        groups
            .set_capacity(slot.id, 6, "Clinic coordinator", &sys())
            .await
            .unwrap();
        let mut tasks = Vec::new();
        for _ in 0..25 {
            let scheduling = s.scheduling.clone();
            let person = Uuid::new_v4();
            tasks.push(tokio::spawn(async move {
                scheduling.book_slot(slot.id, person, &sys()).await
            }));
        }
        let mut won = 0;
        for t in tasks {
            if t.await.expect("a booking task panicked").is_ok() {
                won += 1;
            }
        }
        assert_eq!(won, 6, "round {round}: {won} bookings for six places");
        assert_eq!(live(&s.db, slot.id).await, 6, "round {round}");
        let session = groups.session(slot.id).await.unwrap();
        assert_eq!(
            (session.capacity, session.booked, session.attendees.len()),
            (6, 6, 6)
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_one_to_one_slot_still_holds_one_appointment() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 430).await;
    s.scheduling
        .book_slot(slot.id, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    assert!(
        s.scheduling
            .book_slot(slot.id, Uuid::new_v4(), &sys())
            .await
            .is_err()
    );
    assert_eq!(live(&s.db, slot.id).await, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn each_attendee_has_their_own_outcome_and_a_cancellation_frees_one_place() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let groups = GroupSessionService::new(s.db.clone());
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 432).await;
    groups
        .set_capacity(slot.id, 2, "Clinic coordinator", &sys())
        .await
        .unwrap();
    let a = s
        .scheduling
        .book_slot(slot.id, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    let b = s
        .scheduling
        .book_slot(slot.id, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    assert!(
        s.scheduling
            .book_slot(slot.id, Uuid::new_v4(), &sys())
            .await
            .is_err(),
        "full"
    );

    // One attendee cancels: the other is still booked, and one place opens.
    s.scheduling
        .cancel(a.id, CancellationReason::PatientRequest, &sys())
        .await
        .unwrap();
    let session = groups.session(slot.id).await.unwrap();
    let status = |id: Uuid| {
        session
            .attendees
            .iter()
            .find(|x| x.appointment_id == id)
            .unwrap()
            .status
            .clone()
    };
    assert_eq!(
        (status(a.id).as_str(), status(b.id).as_str()),
        ("cancelled", "booked")
    );
    assert_eq!(session.booked, 1);
    s.scheduling
        .book_slot(slot.id, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    assert!(
        s.scheduling
            .book_slot(slot.id, Uuid::new_v4(), &sys())
            .await
            .is_err(),
        "full again"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_capacity_is_declared_by_a_named_person_and_never_below_those_booked() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let groups = GroupSessionService::new(s.db.clone());
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, 434).await;
    for (cap, by) in [
        (0, "Clinic coordinator"),
        (51, "Clinic coordinator"),
        (4, "  "),
    ] {
        assert!(
            groups.set_capacity(slot.id, cap, by, &sys()).await.is_err(),
            "{cap} {by:?}"
        );
    }
    groups
        .set_capacity(slot.id, 4, "Clinic coordinator", &sys())
        .await
        .unwrap();
    for _ in 0..3 {
        s.scheduling
            .book_slot(slot.id, Uuid::new_v4(), &sys())
            .await
            .unwrap();
    }
    assert!(
        groups
            .set_capacity(slot.id, 2, "Clinic coordinator", &sys())
            .await
            .is_err(),
        "3 booked"
    );
    // Down to exactly those booked: the session is full.
    groups
        .set_capacity(slot.id, 3, "Clinic coordinator", &sys())
        .await
        .unwrap();
    assert!(
        s.scheduling
            .book_slot(slot.id, Uuid::new_v4(), &sys())
            .await
            .is_err()
    );
    // And back up: a place opens.
    groups
        .set_capacity(slot.id, 4, "Clinic coordinator", &sys())
        .await
        .unwrap();
    s.scheduling
        .book_slot(slot.id, Uuid::new_v4(), &sys())
        .await
        .unwrap();

    let audit =
        s.db.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM audit_log WHERE entity_id = $1 AND action = 'set_capacity'",
            [slot.id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audit.try_get::<i64>("", "n").unwrap(), 3);
}
