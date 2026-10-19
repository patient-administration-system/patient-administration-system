//! CAP-13: WhatsApp needs an opt-in and an approved template.
//! Needs PostgreSQL: `cargo test -- --ignored --test-threads=1` (it sets `PAS_WHATSAPP_REQUIRE_OPTIN`).

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::communication::{DeliveryChannel, LetterTemplate};
use patient_administration_system::upstream::{Person, StubPersonClient};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
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
    data.insert(
        "telecom".into(),
        serde_json::json!([{ "system": "phone", "value": phone }]),
    );
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::WhatsApp, Arc::new(Capture(sent.clone())));
    let mut t = LetterTemplate::new(
        format!("wa-{}", Uuid::new_v4()),
        "Hi".into(),
        "Hello.".into(),
    );
    t.channels = vec![DeliveryChannel::WhatsApp];
    let template = LetterRepository::create_template(&s.db, &t)
        .await
        .unwrap()
        .id;
    (s, person, template, sent)
}

async fn send(
    s: &AppState,
    person: Uuid,
    template: Uuid,
) -> patient_administration_system::Result<()> {
    s.communication
        .generate_letter(
            template,
            person,
            None,
            DeliveryChannel::WhatsApp,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .map(|_| ())
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn whatsapp_needs_an_opt_in_and_an_approved_template_when_the_rule_is_on() {
    let (s, person, template, sent) = world().await;
    // SAFETY: single-threaded (see the header).
    unsafe { std::env::set_var("PAS_WHATSAPP_REQUIRE_OPTIN", "true") };

    let err = send(&s, person, template).await.unwrap_err();
    assert!(err.to_string().contains("not opted in"), "{err}");
    s.whatsapp
        .record_optin(person, "in_person", "Clerk A", &sys())
        .await
        .unwrap();
    let err = send(&s, person, template).await.unwrap_err();
    assert!(
        err.to_string().contains("not been approved"),
        "opted in but the template is not approved: {err}"
    );
    assert!(sent.lock().unwrap().is_empty(), "nothing was sent");

    s.whatsapp
        .approve_template(template, "hello_reminder_en", "Clerk A", &sys())
        .await
        .unwrap();
    send(&s, person, template).await.unwrap();
    assert_eq!(sent.lock().unwrap().len(), 1);

    // Withdrawing the opt-in, or the approval, stops it at once.
    s.whatsapp.withdraw_optin(person, &sys()).await.unwrap();
    assert!(send(&s, person, template).await.is_err());
    s.whatsapp
        .record_optin(person, "portal", "Clerk A", &sys())
        .await
        .unwrap();
    send(&s, person, template).await.unwrap();
    s.whatsapp.revoke_template(template, &sys()).await.unwrap();
    assert!(send(&s, person, template).await.is_err());

    // Neither rule touches another channel.
    let mut t = LetterTemplate::new(
        format!("sms-{}", Uuid::new_v4()),
        "Hi".into(),
        "Hello.".into(),
    );
    t.channels = vec![DeliveryChannel::Print];
    let print = LetterRepository::create_template(&s.db, &t)
        .await
        .unwrap()
        .id;
    s.communication
        .generate_letter(
            print,
            person,
            None,
            DeliveryChannel::Print,
            serde_json::json!({}),
            &sys(),
        )
        .await
        .unwrap();
    unsafe { std::env::remove_var("PAS_WHATSAPP_REQUIRE_OPTIN") };
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_rule_is_off_unless_switched_on_and_stop_still_silences_whatsapp() {
    let (s, person, template, sent) = world().await;
    unsafe { std::env::remove_var("PAS_WHATSAPP_REQUIRE_OPTIN") };
    send(&s, person, template)
        .await
        .expect("off: sends as before");
    assert_eq!(sent.lock().unwrap().len(), 1);

    // With everything recorded, a STOP still wins.
    unsafe { std::env::set_var("PAS_WHATSAPP_REQUIRE_OPTIN", "true") };
    s.whatsapp
        .record_optin(person, "patient_reply", "Clerk A", &sys())
        .await
        .unwrap();
    s.whatsapp
        .approve_template(template, "tpl_1", "Clerk A", &sys())
        .await
        .unwrap();
    patient_administration_system::communication::consent::record_optout(
        &s.db,
        &patient_administration_system::communication::consent::person_subject(person),
        "whatsapp",
        "staff",
        None,
    )
    .await
    .unwrap();
    assert!(
        send(&s, person, template)
            .await
            .unwrap_err()
            .to_string()
            .contains("opted out")
    );
    unsafe { std::env::remove_var("PAS_WHATSAPP_REQUIRE_OPTIN") };
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn recording_needs_names_a_known_source_and_a_real_template() {
    let (s, person, template, _) = world().await;
    assert!(
        s.whatsapp
            .record_optin(person, "telepathy", "A", &sys())
            .await
            .is_err()
    );
    assert!(
        s.whatsapp
            .record_optin(person, "paper", " ", &sys())
            .await
            .is_err()
    );
    assert!(
        s.whatsapp.withdraw_optin(person, &sys()).await.is_err(),
        "nothing to withdraw"
    );
    assert!(
        s.whatsapp
            .approve_template(template, " ", "A", &sys())
            .await
            .is_err()
    );
    assert!(
        s.whatsapp
            .approve_template(template, "tpl", " ", &sys())
            .await
            .is_err()
    );
    assert!(
        s.whatsapp
            .approve_template(Uuid::new_v4(), "tpl", "A", &sys())
            .await
            .is_err(),
        "no such template"
    );
    assert!(
        s.whatsapp.revoke_template(template, &sys()).await.is_err(),
        "nothing to revoke"
    );
    assert_eq!(
        common::audit_count(&s.db, person, "whatsapp_optin_recorded").await,
        0
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_http_endpoints_record_and_withdraw() {
    let (s, person, template, _) = world().await;
    let app = patient_administration_system::api::rest::router(s);
    let call = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let mut r = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    r = r.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(r.body(body).unwrap()).await.unwrap();
            let st = r.status();
            let _ = to_bytes(r.into_body(), 1 << 20).await;
            st
        }
    };
    let p = format!("/api/patients/{person}/whatsapp-optin");
    assert_eq!(
        call(
            "PUT",
            p.clone(),
            Some(serde_json::json!({ "source": "paper", "recorded_by": "" }))
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            "PUT",
            p.clone(),
            Some(serde_json::json!({ "source": "paper", "recorded_by": "Clerk A" }))
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(call("DELETE", p.clone(), None).await, StatusCode::OK);
    assert_eq!(call("DELETE", p, None).await, StatusCode::NOT_FOUND);
    let t = format!("/api/letter-templates/{template}/whatsapp-approval");
    assert_eq!(
        call(
            "PUT",
            t.clone(),
            Some(serde_json::json!({ "provider_template_id": "tpl_9", "approved_by": "Clerk A" }))
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(call("DELETE", t, None).await, StatusCode::OK);
}
