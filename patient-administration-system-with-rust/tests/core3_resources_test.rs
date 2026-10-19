//! CORE-3: a room, clinician or piece of equipment cannot be in two places at once.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use chrono::{DateTime, Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::schedule::ScheduleRepository;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::schedule::{Schedule, ScheduleOwner, Slot, SlotStatus};
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

async fn schedule(s: &AppState, owner: ScheduleOwner, service: &str) -> Uuid {
    ScheduleRepository::create(&s.db, &Schedule::new(owner, service.into()))
        .await
        .unwrap()
        .id
}

async fn slot(s: &AppState, schedule: Uuid, start: DateTime<Utc>, mins: i64) -> Uuid {
    SlotRepository::create(
        &s.db,
        &Slot::new(schedule, start, start + Duration::minutes(mins)),
    )
    .await
    .unwrap()
    .id
}

fn when() -> DateTime<Utc> {
    Utc::now() + Duration::days(30 + (Uuid::new_v4().as_u128() % 3000) as i64)
}

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn two_schedules_of_one_room_cannot_both_hold_a_busy_slot_at_the_same_time() {
    let s = state().await;
    let room = Uuid::new_v4();
    let (a, b) = (
        schedule(&s, ScheduleOwner::Room(room), "cardiology").await,
        schedule(&s, ScheduleOwner::Room(room), "dermatology").await,
    );
    let t = when();
    let (sa, sb) = (
        slot(&s, a, t, 30).await,
        slot(&s, b, t + Duration::minutes(15), 30).await,
    );
    s.scheduling
        .book_slot(sa, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    let err = s
        .scheduling
        .book_slot(sb, Uuid::new_v4(), &sys())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already booked"), "{err}");
    // The refused slot is untouched.
    let after = SlotRepository::find_by_id(&s.db, sb)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, SlotStatus::Free);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn different_owners_and_back_to_back_slots_are_fine_and_a_cancel_frees_the_room() {
    let s = state().await;
    let t = when();
    // Different rooms at the same time.
    let (r1, r2) = (
        schedule(&s, ScheduleOwner::Room(Uuid::new_v4()), "x").await,
        schedule(&s, ScheduleOwner::Room(Uuid::new_v4()), "x").await,
    );
    let (s1, s2) = (slot(&s, r1, t, 30).await, slot(&s, r2, t, 30).await);
    s.scheduling
        .book_slot(s1, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    s.scheduling
        .book_slot(s2, Uuid::new_v4(), &sys())
        .await
        .unwrap();

    // The same room, one slot ending exactly when the next starts: no overlap.
    let room = Uuid::new_v4();
    let (a, b) = (
        schedule(&s, ScheduleOwner::Room(room), "x").await,
        schedule(&s, ScheduleOwner::Room(room), "y").await,
    );
    let (sa, sb) = (
        slot(&s, a, t, 30).await,
        slot(&s, b, t + Duration::minutes(30), 30).await,
    );
    let appt = s
        .scheduling
        .book_slot(sa, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    s.scheduling
        .book_slot(sb, Uuid::new_v4(), &sys())
        .await
        .unwrap();

    // Cancelling frees the room for an overlapping booking through the other schedule.
    let sc = slot(&s, b, t, 30).await;
    assert!(
        s.scheduling
            .book_slot(sc, Uuid::new_v4(), &sys())
            .await
            .is_err()
    );
    s.scheduling
        .cancel(
            appt.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &sys(),
        )
        .await
        .unwrap();
    s.scheduling
        .book_slot(sc, Uuid::new_v4(), &sys())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_clinician_and_a_piece_of_equipment_are_resources_too() {
    let s = state().await;
    let t = when();
    for owner in [
        ScheduleOwner::Practitioner(Uuid::new_v4()),
        ScheduleOwner::Equipment(Uuid::new_v4()),
        ScheduleOwner::Bed(Uuid::new_v4()),
    ] {
        let (a, b) = (
            schedule(&s, owner.clone(), "x").await,
            schedule(&s, owner.clone(), "y").await,
        );
        let (sa, sb) = (slot(&s, a, t, 30).await, slot(&s, b, t, 30).await);
        s.scheduling
            .book_slot(sa, Uuid::new_v4(), &sys())
            .await
            .unwrap();
        assert!(
            s.scheduling
                .book_slot(sb, Uuid::new_v4(), &sys())
                .await
                .is_err(),
            "{owner:?}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn equipment_is_a_stored_owner_kind() {
    let s = state().await;
    let eq = Uuid::new_v4();
    let id = schedule(&s, ScheduleOwner::Equipment(eq), "mri").await;
    let back = ScheduleRepository::find_by_id(&s.db, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back.owner, ScheduleOwner::Equipment(eq));
    let by_owner = ScheduleRepository::list_by_owner(&s.db, "equipment", eq)
        .await
        .unwrap();
    assert_eq!(by_owner.len(), 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn two_bookings_arriving_together_through_two_schedules_cannot_both_win() {
    let s = state().await;
    let room = Uuid::new_v4();
    let (a, b) = (
        schedule(&s, ScheduleOwner::Room(room), "x").await,
        schedule(&s, ScheduleOwner::Room(room), "y").await,
    );
    let t = when();
    let (sa, sb) = (slot(&s, a, t, 30).await, slot(&s, b, t, 30).await);
    let ctx = sys();
    let (p, q) = (Uuid::new_v4(), Uuid::new_v4());
    let (r1, r2) = tokio::join!(
        s.scheduling.book_slot(sa, p, &ctx),
        s.scheduling.book_slot(sb, q, &ctx)
    );
    assert_eq!(
        [r1.is_ok(), r2.is_ok()].iter().filter(|x| **x).count(),
        1,
        "exactly one booking wins"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn moving_a_patient_into_a_taken_room_is_refused_and_leaves_them_where_they_were() {
    let s = state().await;
    let room = Uuid::new_v4();
    let (a, b) = (
        schedule(&s, ScheduleOwner::Room(room), "x").await,
        schedule(&s, ScheduleOwner::Room(room), "y").await,
    );
    let t = when();
    let (own, taken, target) = (
        slot(&s, a, t, 30).await,
        slot(&s, a, t + Duration::hours(3), 30).await,
        slot(&s, b, t + Duration::hours(3), 30).await,
    );
    let person = Uuid::new_v4();
    let appt = s.scheduling.book_slot(own, person, &sys()).await.unwrap();
    s.scheduling
        .book_slot(taken, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    assert!(s.scheduling.rebook(appt.id, target, &sys()).await.is_err());
    let now = patient_administration_system::db::repositories::appointment::AppointmentRepository::find_by_id(&s.db, appt.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        now.status,
        patient_administration_system::models::appointment::AppointmentStatus::Booked
    );
}
