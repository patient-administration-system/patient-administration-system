//! An "R" text reply is answered with real alternative times (CAP-5), not left for a person, when there are
//! some. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use patient_administration_system::api::rest::AppState;
use patient_administration_system::contact::ProfileInput;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::inbound::InboundText;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

fn number() -> String {
    format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000)
}

fn text(from: &str, body: &str) -> InboundText {
    InboundText {
        message_id: format!("m-{}", Uuid::new_v4()),
        from: from.into(),
        body: body.into(),
        carrier: Some("test".into()),
        channel: None,
        received_at: None,
    }
}

/// A patient with a mobile and a reminded, booked appointment in a service of its own, plus `others` free slots.
async fn world(others: &[i64], welsh: bool) -> (AppState, String, Uuid, Appointment) {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    let person = Uuid::new_v4();
    let num = number();
    s.contact
        .put(
            person,
            ProfileInput {
                mobile: Some(num.clone()),
                preferred_language: welsh.then(|| "cy-001".to_string()),
                ..Default::default()
            },
            &sys(),
        )
        .await
        .unwrap();
    let own = common::free_slot(&s.db, schedule, 5).await;
    let appt = s
        .scheduling
        .book_slot(own.id, person, &sys())
        .await
        .unwrap();
    for d in others {
        common::free_slot(&s.db, schedule, *d).await;
    }
    s.db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO generated_letters (id, template_id, person_id, appointment_id, rendered_subject, rendered_body, channel, status, sent_at, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,'Reminder','Hello','sms','sent',now(),now(),now())",
        [Uuid::new_v4().into(), Uuid::new_v4().into(), person.into(), appt.id.into()],
    ))
    .await
    .unwrap();
    (s, num, person, appt)
}

async fn status(s: &AppState, id: Uuid) -> AppointmentStatus {
    AppointmentRepository::find_by_id(&s.db, id)
        .await
        .unwrap()
        .unwrap()
        .status
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn r_offers_real_times_and_a_number_books_one_and_the_old_slot_is_released() {
    let (s, num, _, appt) = world(&[6, 7, 8], false).await;
    let o = s.inbound.handle(text(&num, "R"), &sys()).await.unwrap();
    assert_eq!(o.outcome, "asked");
    let reply = o.reply_text.unwrap();
    assert!(
        reply.contains("1 ") && reply.contains("2 ") && reply.contains("stays as it is"),
        "{reply}"
    );
    assert_eq!(
        status(&s, appt.id).await,
        AppointmentStatus::Booked,
        "offering moves nothing"
    );

    let o = s.inbound.handle(text(&num, "2"), &sys()).await.unwrap();
    assert_eq!(o.outcome, "applied");
    assert!(o.reply_text.unwrap().contains("has moved to"));
    assert_eq!(
        status(&s, appt.id).await,
        AppointmentStatus::Cancelled,
        "the old booking is released"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_number_that_is_not_offered_changes_nothing_and_welsh_gets_welsh() {
    let (s, num, _, appt) = world(&[6, 7], true).await;
    let o = s.inbound.handle(text(&num, "R"), &sys()).await.unwrap();
    assert!(o.reply_text.unwrap().contains("Amserau eraill"), "Welsh");
    let o = s.inbound.handle(text(&num, "7"), &sys()).await.unwrap();
    assert_eq!(o.outcome, "unknown");
    assert_eq!(status(&s, appt.id).await, AppointmentStatus::Booked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn with_nothing_to_offer_it_still_goes_to_a_person() {
    let (s, num, _, appt) = world(&[], false).await;
    let o = s.inbound.handle(text(&num, "R"), &sys()).await.unwrap();
    assert_eq!(o.outcome, "needs_staff");
    assert!(o.reply_text.unwrap().contains("will contact you"));
    assert_eq!(status(&s, appt.id).await, AppointmentStatus::Booked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_slot_taken_meanwhile_is_said_plainly_and_the_appointment_stays() {
    let (s, num, _, appt) = world(&[6], false).await;
    s.inbound.handle(text(&num, "R"), &sys()).await.unwrap();
    // Someone else takes the only offered slot before the patient answers.
    s.db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE slots SET status = 'busy' WHERE id IN (SELECT slot_id FROM appointments WHERE id <> $1 AND false) OR (schedule_id = (SELECT sl.schedule_id FROM slots sl WHERE sl.id = (SELECT slot_id FROM appointments WHERE id = $1)) AND id <> (SELECT slot_id FROM appointments WHERE id = $1))",
        [appt.id.into()],
    ))
    .await
    .unwrap();
    let o = s.inbound.handle(text(&num, "1"), &sys()).await.unwrap();
    assert_eq!(o.outcome, "rejected");
    assert_eq!(status(&s, appt.id).await, AppointmentStatus::Booked);
}
