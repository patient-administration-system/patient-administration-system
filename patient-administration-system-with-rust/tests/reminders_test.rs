//! Appointment reminders against a real Postgres. Skips silently without `DATABASE_URL`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::communication::LogMessageSender;
use patient_administration_system::communication::reminders::ReminderService;
use patient_administration_system::db::entities::generated_letter;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use std::sync::Arc;
use uuid::Uuid;

fn person(telecom: serde_json::Value) -> Person {
    let mut data = serde_json::Map::new();
    data.insert(
        "name".into(),
        serde_json::json!({ "given": "Ada", "family": "Lovelace" }),
    );
    data.insert("telecom".into(), telecom);
    Person {
        id: Uuid::new_v4(),
        data,
    }
}

async fn letters_for(db: &sea_orm::DatabaseConnection, appointment_id: Uuid) -> u64 {
    generated_letter::Entity::find()
        .filter(generated_letter::Column::AppointmentId.eq(appointment_id))
        .count(db)
        .await
        .unwrap()
}

async fn book(db: &sea_orm::DatabaseConnection, person_id: Uuid, in_hours: i64) -> Appointment {
    let start = Utc::now() + Duration::hours(in_hours);
    let mut a = Appointment::new(person_id, start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    a.reason = Some("Knee review".into());
    AppointmentRepository::create(db, &a).await.unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn reminders_send_once_skip_unreachable_and_ignore_far_future() {
    let Some(url) = common::database_url() else {
        return;
    };
    let state = common::build_state(&url).await;

    let reachable = person(serde_json::json!([{ "system": "phone", "value": "+15555550100" }]));
    let unreachable = person(serde_json::json!([]));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(reachable.clone());
    stub.insert(unreachable.clone());

    let state = state.with_person_client(stub).with_sender(
        DeliveryChannel::WhatsApp,
        Arc::new(LogMessageSender {
            channel: "whatsapp",
        }),
    );
    let service = ReminderService::new(state.db.clone(), state.communication.clone());

    let soon = book(&state.db, reachable.id, 2).await;
    let no_contact = book(&state.db, unreachable.id, 3).await;
    let far = book(&state.db, reachable.id, 24 * 10).await;

    let first = service
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap();
    assert!(first.sent >= 1);
    assert_eq!(
        letters_for(&state.db, soon.id).await,
        1,
        "reachable patient is reminded once"
    );
    assert_eq!(
        letters_for(&state.db, no_contact.id).await,
        0,
        "no contact point, nothing sent"
    );
    assert_eq!(
        letters_for(&state.db, far.id).await,
        0,
        "outside the lead window"
    );

    // The patient has no language preference, so the English variant is the one used.
    let template = ReminderService::new(state.db.clone(), state.communication.clone())
        .ensure_default_template()
        .await
        .unwrap();
    assert!(
        LetterRepository::exists_for_appointment(&state.db, soon.id, template.id)
            .await
            .unwrap()
    );

    // A second scan must not send again.
    service
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap();
    assert_eq!(letters_for(&state.db, soon.id).await, 1, "no double send");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn edited_template_wording_is_what_gets_sent() {
    let Some(url) = common::database_url() else {
        return;
    };
    let state = common::build_state(&url).await;
    let p = person(serde_json::json!([{ "system": "phone", "value": "+15555550101" }]));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(p.clone());
    let state = state.with_person_client(stub).with_sender(
        DeliveryChannel::WhatsApp,
        Arc::new(LogMessageSender {
            channel: "whatsapp",
        }),
    );
    let service = ReminderService::new(state.db.clone(), state.communication.clone());

    let mut t = service.ensure_default_template().await.unwrap();
    t.body_tera = "EDITED for {{ patient.name.given }}".into();
    LetterRepository::update_template(&state.db, &t)
        .await
        .unwrap();

    let appt = book(&state.db, p.id, 4).await;
    service
        .run_once(Utc::now(), Duration::hours(24))
        .await
        .unwrap();

    let letter = generated_letter::Entity::find()
        .filter(generated_letter::Column::AppointmentId.eq(appt.id))
        .one(&state.db)
        .await
        .unwrap()
        .expect("a reminder was generated");
    assert_eq!(letter.rendered_body, "EDITED for Ada");
    assert_eq!(letter.channel, "whatsapp");
    assert_eq!(letter.status, "sent");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn portal_channel_stores_and_delivers_without_a_carrier_and_exposes_portal_url() {
    let Some(url) = common::database_url() else {
        return;
    };
    let state = common::build_state(&url).await;
    // A patient with no phone or email at all: the portal needs neither.
    let p = person(serde_json::json!([]));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(p.clone());
    let state = state
        .with_person_client(stub)
        .with_portal_base_url("https://pas.example.test/");
    assert_eq!(
        state.communication.portal_base_url(),
        Some("https://pas.example.test")
    );

    let mut t = patient_administration_system::models::communication::LetterTemplate::new(
        format!("portal-test-{}", Uuid::new_v4()),
        "New message".into(),
        "You have a new message. Sign in to read it: {{ portal_url }}".into(),
    );
    t.channels = vec![DeliveryChannel::Portal];
    let t = LetterRepository::create_template(&state.db, &t)
        .await
        .unwrap();

    let letter = state
        .communication
        .generate_letter(
            t.id,
            p.id,
            None,
            DeliveryChannel::Portal,
            serde_json::json!({}),
            &patient_administration_system::db::repositories::audit::UserContext::system(),
        )
        .await
        .unwrap();
    assert_eq!(letter.channel, DeliveryChannel::Portal);
    assert_eq!(
        letter.status,
        patient_administration_system::models::communication::LetterStatus::Sent
    );
    assert_eq!(
        letter.rendered_body,
        "You have a new message. Sign in to read it: https://pas.example.test/my/messages"
    );

    let mine = LetterRepository::list_for_person(&state.db, p.id, Some(DeliveryChannel::Portal))
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    let none = LetterRepository::list_for_person(&state.db, p.id, Some(DeliveryChannel::Sms))
        .await
        .unwrap();
    assert!(none.is_empty(), "channel filter excludes other channels");
}
