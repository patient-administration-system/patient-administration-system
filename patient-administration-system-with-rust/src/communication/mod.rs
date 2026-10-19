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

pub mod consent;
pub mod ladder;
pub mod reminders;
pub mod sender;
pub mod sms;
pub mod sms_length;
pub mod templates;

pub use sender::{LogMessageSender, MessageSender, SmsSender};
pub use sms::{LogSmsProvider, NoopSmsProvider, SmsProvider};

use sea_orm::{DatabaseConnection, TransactionTrait};
use std::collections::HashMap;
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
    senders: HashMap<DeliveryChannel, Arc<dyn MessageSender>>,
    portal_base_url: Option<String>,
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
            senders: HashMap::new(),
            portal_base_url: None,
            person_client: None,
        }
    }

    /// A copy of this service with the same configuration: database, publisher, SMS provider,
    /// every sender, the portal address and the person client. Builders start from this so that
    /// changing one thing can never drop another.
    pub fn derive(&self) -> CommunicationService {
        CommunicationService {
            db: self.db.clone(),
            publisher: self.publisher.clone(),
            sms_provider: self.sms_provider.clone(),
            senders: self.senders.clone(),
            portal_base_url: self.portal_base_url.clone(),
            person_client: self.person_client.clone(),
        }
    }

    /// Public address of the patient portal (no trailing slash), e.g.
    /// `https://pas.example.nhs.uk`. Exposed to templates as `portal_url`.
    pub fn with_portal_base_url(mut self, url: impl Into<String>) -> Self {
        self.portal_base_url = Some(url.into().trim_end_matches('/').to_string());
        self
    }

    pub fn portal_base_url(&self) -> Option<&str> {
        self.portal_base_url.as_deref()
    }

    /// Wire a sender for Email, AppleMessages or WhatsApp. SMS keeps using
    /// [`Self::with_sms_provider`].
    pub fn with_sender(mut self, channel: DeliveryChannel, sender: Arc<dyn MessageSender>) -> Self {
        self.senders.insert(channel, sender);
        self
    }

    pub fn with_sms_provider(mut self, provider: Arc<dyn SmsProvider>) -> Self {
        self.sms_provider = provider.clone();
        self.senders
            .insert(DeliveryChannel::Sms, Arc::new(SmsSender(provider)));
        self
    }

    pub fn with_person_client(mut self, client: Arc<dyn PersonClient>) -> Self {
        self.person_client = Some(client);
        self
    }

    /// Read-only accessor for the current SMS provider — `AppState`
    /// uses it when rebuilding the service with a new PersonClient
    /// to carry over whatever SMS provider was previously installed.
    /// The address `channel` would deliver to for `person_id`, or `None` when
    /// the person has no suitable contact point. Lets callers skip a channel
    /// before any letter is persisted.
    pub async fn recipient(
        &self,
        person_id: Uuid,
        channel: DeliveryChannel,
        ctx: &UserContext,
    ) -> Result<Option<String>> {
        let person_client = self
            .person_client
            .as_ref()
            .ok_or_else(|| Error::upstream("CommunicationService not wired with a PersonClient"))?;
        let upstream_ctx = UpstreamUserContext {
            user_id: ctx.user_id.clone(),
            user_ip: ctx.user_ip.clone(),
            user_agent: ctx.user_agent.clone(),
        };
        let person = person_client.get(person_id, &upstream_ctx).await?;
        let value = serde_json::to_value(&person).unwrap_or_default();
        self.resolve_recipient(channel, person_id, &value).await
    }

    /// Where a message on `channel` should go. For phone-based channels a *verified, recent* profile
    /// mobile wins over the upstream record, because that is the number the patient confirmed;
    /// anything less falls back to upstream, so an unproven number never receives a message.
    async fn resolve_recipient(
        &self,
        channel: DeliveryChannel,
        person_id: Uuid,
        upstream_person: &serde_json::Value,
    ) -> Result<Option<String>> {
        if matches!(
            channel,
            DeliveryChannel::Sms | DeliveryChannel::WhatsApp | DeliveryChannel::AppleMessages
        ) && let Some(mobile) = crate::contact::trusted_mobile(&self.db, person_id).await?
        {
            return Ok(Some(mobile));
        }
        Ok(recipient_for(channel, upstream_person))
    }

    /// True when a sender is wired for `channel` and reports itself enabled.
    pub fn channel_ready(&self, channel: DeliveryChannel) -> bool {
        self.senders.get(&channel).is_some_and(|s| s.is_enabled())
    }

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

        // 1b. Never send to someone who has opted out of this channel (PILOT-7). This is the choke
        //     point every send passes through, so no caller can skip it.
        let person_json = serde_json::to_value(&person).unwrap_or_default();
        let address = self
            .resolve_recipient(channel, person_id, &person_json)
            .await?;
        if consent::is_opted_out(&self.db, person_id, channel, address.as_deref()).await? {
            return Err(Error::conflict(format!(
                "the recipient has opted out of {} messages",
                channel_label(channel)
            )));
        }

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
        // A link to the secure portal, for messages that must not carry the
        // detail themselves ("You have a new message: {{ portal_url }}").
        if let Some(base) = &self.portal_base_url {
            tctx.insert("portal_url", &format!("{base}/my/messages"));
        }
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
        letter.locale = Some(template.locale.clone());

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
        // The portal needs no carrier: storing the message is delivering it.
        if channel == DeliveryChannel::Portal {
            let updated = LetterRepository::mark_sent(&self.db, persisted.id).await?;
            let _ = AuditLogRepository::log(
                &self.db,
                "generated_letter",
                updated.id,
                "deliver_portal",
                None,
                None,
                ctx,
            )
            .await;
            return Ok(updated);
        }

        let sender = self.senders.get(&channel).filter(|s| s.is_enabled());
        let letter = match sender {
            None => persisted,
            Some(sender) => {
                let label = channel_label(channel);
                match self
                    .resolve_recipient(channel, person_id, &person_value)
                    .await?
                {
                    None => {
                        tracing::warn!(
                            target: "pas::message",
                            letter_id = %persisted.id,
                            person_id = %person_id,
                            channel = label,
                            "letter generated but upstream person has no contact point for the channel; leaving status=pending"
                        );
                        let _ = AuditLogRepository::log(
                            &self.db,
                            "generated_letter",
                            persisted.id,
                            &format!("send_{label}_skipped_no_recipient"),
                            None,
                            None,
                            ctx,
                        )
                        .await;
                        persisted
                    }
                    Some(to) => match sender
                        .send(&to, &persisted.rendered_subject, &persisted.rendered_body)
                        .await
                    {
                        Ok(()) => match LetterRepository::mark_sent(&self.db, persisted.id).await {
                            Ok(updated) => {
                                let _ = AuditLogRepository::log(
                                    &self.db,
                                    "generated_letter",
                                    updated.id,
                                    &format!("send_{label}_ok"),
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
                                &format!("send_{label}_failed"),
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
            }
        };

        let _ = LetterStatus::Pending;
        Ok(letter)
    }
}

/// Pluck the first phone telecom from an upstream Person JSON shape.
/// The upstream emits FHIR-flavored `telecom: [{ system, value, … }]`
/// arrays; this helper is intentionally tolerant of either FHIR-style
/// `"phone"` or PAS-style `"Phone"` system strings.
/// Refuse a template whose subject or body is not valid Tera, so a typo is caught when an
/// administrator saves it and not when a patient's reminder fails to render (FR-7).
pub fn validate_template_syntax(subject: &str, body_tera: &str) -> Result<()> {
    for (field, src) in [("subject", subject), ("body_tera", body_tera)] {
        if let Err(e) = tera::Tera::default().add_raw_template("t", src) {
            return Err(Error::validation(format!("{field}: {e}")));
        }
    }
    Ok(())
}

fn channel_label(channel: DeliveryChannel) -> &'static str {
    match channel {
        DeliveryChannel::Print => "print",
        DeliveryChannel::Email => "email",
        DeliveryChannel::Sms => "sms",
        DeliveryChannel::AppleMessages => "apple_messages",
        DeliveryChannel::WhatsApp => "whatsapp",
        DeliveryChannel::Portal => "portal",
    }
}

fn email_from_upstream(person: &serde_json::Value) -> Option<String> {
    let telecom = person.get("telecom")?.as_array()?;
    telecom.iter().find_map(|cp| {
        let system = cp
            .get("system")
            .and_then(|v| v.as_str())?
            .to_ascii_lowercase();
        if system == "email" {
            cp.get("value").and_then(|v| v.as_str()).map(String::from)
        } else {
            None
        }
    })
}

/// The address a channel delivers to, or `None` when the person has no
/// suitable contact point (or the channel has no remote recipient, i.e. print).
pub(crate) fn recipient_for(
    channel: DeliveryChannel,
    person: &serde_json::Value,
) -> Option<String> {
    match channel {
        DeliveryChannel::Print | DeliveryChannel::Portal => None,
        DeliveryChannel::Email => email_from_upstream(person),
        DeliveryChannel::Sms | DeliveryChannel::WhatsApp => phone_from_upstream(person),
        DeliveryChannel::AppleMessages => {
            phone_from_upstream(person).or_else(|| email_from_upstream(person))
        }
    }
}

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
    fn test_template_syntax_is_checked_for_subject_and_body() {
        assert!(
            validate_template_syntax(
                "Hello {{ patient.name.given }}",
                "Seen at {{ appointment.start }}"
            )
            .is_ok()
        );
        assert!(validate_template_syntax("plain", "also plain").is_ok());
        let bad_body = validate_template_syntax("ok", "{{ unclosed")
            .unwrap_err()
            .to_string();
        assert!(bad_body.contains("body_tera"), "{bad_body}");
        assert!(
            validate_template_syntax("{% if %}", "ok")
                .unwrap_err()
                .to_string()
                .contains("subject")
        );
        assert!(validate_template_syntax("ok", "{% for x in %}{% endfor %}").is_err());
    }

    #[test]
    fn test_recipient_for_channels() {
        let p = serde_json::json!({
            "telecom": [
                { "system": "email", "value": "a@b.io" },
                { "system": "phone", "value": "555-1234" },
            ]
        });
        assert_eq!(
            recipient_for(DeliveryChannel::Email, &p),
            Some("a@b.io".into())
        );
        assert_eq!(
            recipient_for(DeliveryChannel::Sms, &p),
            Some("555-1234".into())
        );
        assert_eq!(
            recipient_for(DeliveryChannel::WhatsApp, &p),
            Some("555-1234".into())
        );
        assert_eq!(
            recipient_for(DeliveryChannel::AppleMessages, &p),
            Some("555-1234".into())
        );
        assert_eq!(recipient_for(DeliveryChannel::Print, &p), None);
        let email_only =
            serde_json::json!({ "telecom": [{ "system": "email", "value": "a@b.io" }] });
        assert_eq!(
            recipient_for(DeliveryChannel::AppleMessages, &email_only),
            Some("a@b.io".into())
        );
        assert_eq!(recipient_for(DeliveryChannel::WhatsApp, &email_only), None);
    }

    #[test]
    fn test_phone_from_upstream_returns_none_when_telecom_missing() {
        let p = serde_json::json!({ "name": { "family": "Doe" } });
        assert!(phone_from_upstream(&p).is_none());
    }
}
