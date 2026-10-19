//! CAS-10: a patient who can only be reached by letter is never sent a digital message.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use async_trait::async_trait;
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::contact::ProfileInput;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::communication::{DeliveryChannel, LetterTemplate};
use patient_administration_system::upstream::{Person, StubPersonClient};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct Capture(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl MessageSender for Capture {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _s: &str,
        _b: &str,
    ) -> patient_administration_system::Result<()> {
        self.0.lock().unwrap().push(to.to_string());
        Ok(())
    }
}

async fn world() -> (AppState, Uuid, Uuid, Arc<Mutex<Vec<String>>>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let person = Uuid::new_v4();
    let phone = format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000).replace(' ', "");
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    data.insert("telecom".into(), serde_json::json!([{ "system": "phone", "value": phone }, { "system": "email", "value": format!("{person}@example.test") }]));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture(sent.clone())))
        .with_sender(DeliveryChannel::Email, Arc::new(Capture(sent.clone())))
        .with_sender(DeliveryChannel::WhatsApp, Arc::new(Capture(sent.clone())));
    let mut t = LetterTemplate::new(
        format!("lo-{}", Uuid::new_v4()),
        "Hello".into(),
        "Your appointment.".into(),
    );
    t.channels = vec![
        DeliveryChannel::Sms,
        DeliveryChannel::Email,
        DeliveryChannel::WhatsApp,
        DeliveryChannel::Portal,
        DeliveryChannel::Print,
    ];
    let template = LetterRepository::create_template(&s.db, &t)
        .await
        .unwrap()
        .id;
    (s, person, template, sent)
}

async fn set_needs(s: &AppState, person: Uuid, needs: &[&str]) {
    s.contact
        .put(
            person,
            ProfileInput {
                mobile: None,
                alternate_name: None,
                alternate_relationship: None,
                alternate_phone: None,
                alternate_email: None,
                alternate_is_carer: false,
                alternate_is_emergency: false,
                preferred_channel: None,
                preferred_language: None,
                accessibility_needs: needs.iter().map(|n| n.to_string()).collect(),
            },
            &sys(),
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_letter_only_patient_is_never_sent_a_digital_message_but_a_letter_is_made() {
    let (s, person, template, sent) = world().await;
    set_needs(&s, person, &["letter_only", "large_print"]).await;
    for channel in [
        DeliveryChannel::Sms,
        DeliveryChannel::Email,
        DeliveryChannel::WhatsApp,
        DeliveryChannel::Portal,
    ] {
        let err = s
            .communication
            .generate_letter(
                template,
                person,
                None,
                channel,
                serde_json::json!({}),
                &sys(),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("only be contacted by letter"),
            "{channel:?}: {err}"
        );
    }
    assert!(sent.lock().unwrap().is_empty(), "nothing was sent");
    // No message record was made for the refused channels either.
    assert_eq!(
        LetterRepository::list_for_person(&s.db, person, None)
            .await
            .unwrap()
            .len(),
        0
    );
    // A letter is fine.
    let l = s
        .communication
        .generate_letter(
            template,
            person,
            None,
            DeliveryChannel::Print,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(l.channel, DeliveryChannel::Print);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn other_patients_and_other_needs_are_unaffected_and_the_flag_can_be_cleared() {
    let (s, person, template, sent) = world().await;
    // Large print is not letter-only.
    set_needs(&s, person, &["large_print"]).await;
    s.communication
        .generate_letter(
            template,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(sent.lock().unwrap().len(), 1);
    // Set, then clear.
    set_needs(&s, person, &["letter_only"]).await;
    assert!(
        s.communication
            .generate_letter(
                template,
                person,
                None,
                DeliveryChannel::Sms,
                serde_json::json!({}),
                &sys()
            )
            .await
            .is_err()
    );
    set_needs(&s, person, &[]).await;
    s.communication
        .generate_letter(
            template,
            person,
            None,
            DeliveryChannel::Sms,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(sent.lock().unwrap().len(), 2);
}
