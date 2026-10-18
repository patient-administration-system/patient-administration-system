//! communication — Letter template rendering, persistence, and SMS dispatch.
//!
//! v0.44.0 Phase 3b3 — restores `generate_letter` over the upstream
//! `PersonClient`. The renderer:
//!
//! 1. Fetches the upstream `Person` (the demographic snapshot) via
//!    [`PersonClient::get`] using the inbound caller's
//!    `UserContext`, so the upstream's audit log records who initiated
//!    the letter.
//! 2. Binds the Person JSON shape as the Tera context variable named
//!    `patient` (keeping the v0.43.0 template variable name so existing
//!    `{{ patient.name.family }}` templates continue to work). Any
//!    `extra_context` map merges on top.
//! 3. Renders subject + body with strict missing-variable detection.
//! 4. Persists the [`GeneratedLetter`], writes audit + outbox in one
//!    DB transaction.
//! 5. If the channel is SMS and a real provider is wired, looks up the
//!    person's first `telecom` entry with `system == "phone"` from the
//!    upstream JSON, dispatches via [`SmsProvider::send`], and flips
//!    status accordingly.

pub mod sms;

pub use sms::{LogSmsProvider, NoopSmsProvider, SmsProvider};

use sea_orm::{DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::letter::LetterRepository;
use crate::db::repositories::outbox::OutboxRepository;
use crate::models::communication::{DeliveryChannel, GeneratedLetter, LetterStatus};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::upstream::{PersonClient, UserContext as UpstreamUserContext};
use crate::{Error, Result};

pub struct CommunicationService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
    sms_provider: Arc<dyn SmsProvider>,
    /// Upstream identity client. `None` keeps the v0.44.0 Phase 2 stub
    /// behavior (every `generate_letter` returns `Error::Upstream`); the
    /// real renderer needs the upstream client to fetch demographic JSON.
    /// `main.rs` injects the client at startup.
    person_client: Option<Arc<dyn PersonClient>>,
}

impl CommunicationService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self {
            db,
            publisher,
            sms_provider: Arc::new(NoopSmsProvider),
            person_client: None,
        }
    }

    pub fn with_sms_provider(mut self, provider: Arc<dyn SmsProvider>) -> Self {
        self.sms_provider = provider;
        self
    }

    pub fn with_person_client(mut self, client: Arc<dyn PersonClient>) -> Self {
        self.person_client = Some(client);
        self
    }

    /// Read-only accessor for the current SMS provider — `AppState`
    /// uses it when rebuilding the service with a new PersonClient
    /// to carry over whatever SMS provider was previously installed.
    pub fn sms_provider_arc(&self) -> Arc<dyn SmsProvider> {
        self.sms_provider.clone()
    }

    /// Render a letter template against the upstream person snapshot,
    /// persist the result, fire audit + outbox, and (for SMS channels)
    /// dispatch via the configured provider.
    pub async fn generate_letter(
        &self,
        template_id: Uuid,
        person_id: Uuid,
        appointment_id: Option<Uuid>,
        channel: DeliveryChannel,
        extra_context: serde_json::Value,
        ctx: &UserContext,
    ) -> Result<GeneratedLetter> {
        let person_client = self.person_client.as_ref().ok_or_else(|| {
            Error::upstream(
                "CommunicationService not wired with a PersonClient; \
                 set PERSON_SERVICE_URL in main.rs",
            )
        })?;

        // 1. Fetch upstream person (proxies inbound user context so the
        //    upstream audit credits the original caller).
        let upstream_ctx = UpstreamUserContext {
            user_id: ctx.user_id.clone(),
            user_ip: ctx.user_ip.clone(),
            user_agent: ctx.user_agent.clone(),
        };
        let person = person_client.get(person_id, &upstream_ctx).await?;

        // 2. Load template.
        let template = LetterRepository::find_template_by_id(&self.db, template_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("letter template {template_id}")))?;

        // 3. Build the Tera context. `patient` is the historical variable
        //    name; templates that used `{{ patient.name.family }}` keep
        //    working because the upstream Person JSON shape preserves
        //    `name.family`.
        let mut tctx = tera::Context::new();
        let person_value =
            serde_json::to_value(&person).unwrap_or(serde_json::Value::Object(Default::default()));
        tctx.insert("patient", &person_value);
        if let serde_json::Value::Object(map) = &extra_context {
            for (k, v) in map {
                tctx.insert(k, v);
            }
        }

        // 4. Strict missing-variable check.
        for var in &template.required_variables {
            if tctx.get(var).is_none() {
                return Err(Error::validation(format!(
                    "missing required template variable: {var}"
                )));
            }
        }

        // 5. Render.
        let rendered_body = tera::Tera::one_off(&template.body_tera, &tctx, false)
            .map_err(|e| Error::render(format!("body: {e}")))?;
        let rendered_subject = tera::Tera::one_off(&template.subject, &tctx, false)
            .map_err(|e| Error::render(format!("subject: {e}")))?;

        let mut letter = GeneratedLetter::new(
            template_id,
            person_id,
            channel,
            rendered_subject,
            rendered_body,
        );
        letter.appointment_id = appointment_id;

        // 6. Persist + audit + outbox in one DB tx.
        let ctx_clone = ctx.clone();
        let letter_clone = letter.clone();
        let persisted = self
            .db
            .transaction::<_, GeneratedLetter, Error>(|txn| {
                Box::pin(async move {
                    let l = LetterRepository::create_generated_letter(txn, &letter_clone).await?;
                    AuditLogRepository::log(
                        txn,
                        "generated_letter",
                        l.id,
                        "generate",
                        None,
                        Some(serde_json::to_value(&l).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "LetterGenerated",
                        &serde_json::json!({
                            "letter_id": l.id,
                            "template_id": template_id,
                            "person_id": person_id,
                        }),
                    )
                    .await?;
                    Ok(l)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "LetterGenerated",
                serde_json::json!({ "letter_id": persisted.id }),
            ))
            .await;

        // 7. SMS auto-send when configured and the channel is SMS.
        let letter = if matches!(channel, DeliveryChannel::Sms) && self.sms_provider.is_enabled() {
            let phone = phone_from_upstream(&person_value);
            match phone {
                None => {
                    tracing::warn!(
                        target: "pas::sms",
                        letter_id = %persisted.id,
                        person_id = %person_id,
                        "SMS letter generated but upstream person has no Phone telecom; leaving status=pending"
                    );
                    let _ = AuditLogRepository::log(
                        &self.db,
                        "generated_letter",
                        persisted.id,
                        "send_sms_skipped_no_phone",
                        None,
                        None,
                        ctx,
                    )
                    .await;
                    persisted
                }
                Some(to) => match self.sms_provider.send(&to, &persisted.rendered_body).await {
                    Ok(()) => match LetterRepository::mark_sent(&self.db, persisted.id).await {
                        Ok(updated) => {
                            let _ = AuditLogRepository::log(
                                &self.db,
                                "generated_letter",
                                updated.id,
                                "send_sms_ok",
                                None,
                                Some(serde_json::json!({ "to": to })),
                                ctx,
                            )
                            .await;
                            updated
                        }
                        Err(_) => persisted,
                    },
                    Err(send_err) => {
                        let _ = AuditLogRepository::log(
                            &self.db,
                            "generated_letter",
                            persisted.id,
                            "send_sms_failed",
                            None,
                            Some(serde_json::json!({
                                "to": to,
                                "error": send_err.to_string(),
                            })),
                            ctx,
                        )
                        .await;
                        LetterRepository::mark_failed(&self.db, persisted.id)
                            .await
                            .unwrap_or(persisted)
                    }
                },
            }
        } else {
            persisted
        };

        // Silence the unused-import warning when the SMS path isn't taken.
        let _ = LetterStatus::Pending;
        Ok(letter)
    }
}

/// Pluck the first phone telecom from an upstream Person JSON shape.
/// The upstream emits FHIR-flavored `telecom: [{ system, value, … }]`
/// arrays; this helper is intentionally tolerant of either FHIR-style
/// `"phone"` or PAS-style `"Phone"` system strings.
fn phone_from_upstream(person: &serde_json::Value) -> Option<String> {
    let telecom = person.get("telecom")?.as_array()?;
    for cp in telecom {
        let system = cp
            .get("system")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if (system == "phone" || system == "sms")
            && let Some(value) = cp.get("value").and_then(|v| v.as_str())
        {
            return Some(value.to_string());
        }
    }
    None
}

fn unwrap_txn_err(e: sea_orm::TransactionError<Error>) -> Error {
    match e {
        sea_orm::TransactionError::Connection(c) => Error::Database(c),
        sea_orm::TransactionError::Transaction(t) => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tera_render_smoke() {
        let mut ctx = tera::Context::new();
        ctx.insert("name", "World");
        let out = tera::Tera::one_off("Hello {{ name }}", &ctx, false).unwrap();
        assert_eq!(out, "Hello World");
    }

    #[test]
    fn test_phone_from_upstream_finds_phone_lowercase_system() {
        let p = serde_json::json!({
            "telecom": [
                { "system": "email", "value": "a@b.io" },
                { "system": "phone", "value": "555-1234" },
            ]
        });
        assert_eq!(phone_from_upstream(&p), Some("555-1234".into()));
    }

    #[test]
    fn test_phone_from_upstream_finds_phone_capital_system() {
        let p = serde_json::json!({
            "telecom": [
                { "system": "Phone", "value": "555-9999" },
            ]
        });
        assert_eq!(phone_from_upstream(&p), Some("555-9999".into()));
    }

    #[test]
    fn test_phone_from_upstream_returns_none_when_no_phone() {
        let p = serde_json::json!({
            "telecom": [
                { "system": "email", "value": "a@b.io" },
            ]
        });
        assert!(phone_from_upstream(&p).is_none());
    }

    #[test]
    fn test_phone_from_upstream_returns_none_when_telecom_missing() {
        let p = serde_json::json!({ "name": { "family": "Doe" } });
        assert!(phone_from_upstream(&p).is_none());
    }
}
