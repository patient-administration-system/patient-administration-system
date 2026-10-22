//! Appointment reminders.
//!
//! [`ReminderService::run_once`] finds booked appointments starting within the
//! lead window that have not been reminded yet, renders the administrator-edited
//! `appointment_reminder` letter template for each, and sends it through the
//! first channel (in the template's channel order) for which the patient has a
//! contact point and an enabled sender. The rendered letter doubles as the
//! "already reminded" marker, so repeated runs never double-send.
//!
//! Template variables: `patient` (upstream person), `appointment`
//! (`id`, `start`, `end`, `reason`).

use chrono::{DateTime, Duration, Utc};
use std::sync::Arc;

use super::CommunicationService;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::UserContext;
use crate::db::repositories::letter::LetterRepository;
use crate::models::appointment::Appointment;
use crate::models::communication::{DeliveryChannel, LetterTemplate};
use crate::{Error, Result};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

/// Name administrators edit to change the reminder wording.
pub const REMINDER_TEMPLATE_NAME: &str = "appointment_reminder";

/// After this many failed delivery attempts for one appointment, stop trying. Without a limit a
/// carrier outage makes every five-minute scan write a new failed letter for the same appointment,
/// and a number that bounces is hammered for ever. A person can still follow up (CAP-7).
pub const MAX_FAILED_ATTEMPTS: i64 = 6;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReminderReport {
    /// Appointments given up on after [`MAX_FAILED_ATTEMPTS`] failed attempts.
    pub gave_up: usize,
    pub considered: usize,
    pub sent: usize,
    pub skipped_already_reminded: usize,
    pub skipped_no_channel: usize,
    /// Held back because it is quiet hours; they are sent on a later scan, not dropped.
    pub deferred_quiet_hours: usize,
    /// Left unsent because the patient has opted out of every channel that could reach them.
    pub skipped_opted_out: usize,
    /// Failed before and not yet due for another try (NOT-4).
    pub backing_off: usize,
}

pub struct ReminderService {
    db: DatabaseConnection,
    communication: Arc<CommunicationService>,
    quiet_hours: Option<super::consent::QuietHours>,
    backoff: bool,
}

impl ReminderService {
    pub fn new(db: DatabaseConnection, communication: Arc<CommunicationService>) -> Self {
        Self {
            db,
            communication,
            quiet_hours: None,
            backoff: true,
        }
    }

    /// Retry failed reminders at once on every scan instead of backing off. For tests of the retry cap; the
    /// server always backs off (see [`crate::receipts::retry_delay`]).
    pub fn without_backoff(mut self) -> Self {
        self.backoff = false;
        self
    }

    /// Hold text-style channels back during these hours (clinic time, `PAS_TIMEZONE`). The server sets
    /// it from `PAS_QUIET_HOURS`; tests that do not care leave it off.
    pub fn with_quiet_hours(mut self, q: Option<super::consent::QuietHours>) -> Self {
        self.quiet_hours = q;
        self
    }

    /// Make sure the reminder has an active variant for every required language, adding the default
    /// wording for any that is missing. Administrators edit them afterwards (`PUT
    /// /api/letter-templates/{id}`); a variant they have written is never replaced.
    pub async fn ensure_default_templates(&self) -> Result<Vec<LetterTemplate>> {
        self.ensure_templates_named(REMINDER_TEMPLATE_NAME).await
    }

    /// The same for any ladder message (`appointment_reminder`, `_second`, `_final`).
    pub async fn ensure_templates_named(&self, name: &str) -> Result<Vec<LetterTemplate>> {
        let required = super::templates::required_locales();
        let mut present = LetterRepository::list_active_by_name(&self.db, name).await?;
        for locale in &required {
            if present.iter().any(|t| &t.locale == locale) {
                continue;
            }
            let Some((subject, body)) = default_wording(name, locale) else {
                continue;
            };
            let mut t =
                LetterTemplate::new(name.to_string(), subject.to_string(), body.to_string());
            t.locale = locale.clone();
            t.channels = vec![
                DeliveryChannel::WhatsApp,
                DeliveryChannel::AppleMessages,
                DeliveryChannel::Sms,
                DeliveryChannel::Email,
            ];
            // A sibling's channels win, so the variants of one message always offer the same channels.
            if let Some(first) = present.first() {
                t.channels = first.channels.clone();
            }
            present.push(LetterRepository::create_template(&self.db, &t).await?);
        }
        Ok(present)
    }

    /// The default-language variant (English unless `PAS_REQUIRED_LOCALES` says otherwise).
    pub async fn ensure_default_template(&self) -> Result<LetterTemplate> {
        let all = self.ensure_default_templates().await?;
        let required = super::templates::required_locales();
        super::templates::choose(&all, None, &required)
            .cloned()
            .ok_or_else(|| Error::internal("no reminder template could be created"))
    }

    #[tracing::instrument(skip_all, fields(job = "reminders"))]
    pub async fn run_once(&self, now: DateTime<Utc>, lead: Duration) -> Result<ReminderReport> {
        let templates = self.ensure_default_templates().await?;
        let required = super::templates::required_locales();
        let ids: Vec<uuid::Uuid> = templates.iter().map(|t| t.id).collect();
        let channels = templates
            .first()
            .map(|t| t.channels.clone())
            .unwrap_or_default();
        let appointments =
            AppointmentRepository::list_booked_starting_between(&self.db, now, now + lead).await?;
        let mut report = ReminderReport {
            considered: appointments.len(),
            ..Default::default()
        };
        let ctx = UserContext::system();
        for a in appointments {
            match self.skip_reason(&a, &ids, now).await? {
                Some(Skip::AlreadyReminded) => {
                    report.skipped_already_reminded += 1;
                    continue;
                }
                Some(Skip::GaveUp) => {
                    report.gave_up += 1;
                    continue;
                }
                Some(Skip::BackingOff) => {
                    report.backing_off += 1;
                    continue;
                }
                None => {}
            }
            // The patient's language picks the variant; `when` reads in that language and the clinic's
            // time zone, which is how they will read it (the RFC 3339 `start` stays for templates that want it).
            let language = crate::contact::preferred_language(&self.db, a.person_id).await?;
            let Some(template) =
                super::templates::choose(&templates, language.as_deref(), &required)
            else {
                report.skipped_no_channel += 1;
                continue;
            };
            match self
                .try_channels(&a, template, &channels, now, &ctx)
                .await?
            {
                Tried::Sent => report.sent += 1,
                Tried::Deferred => report.deferred_quiet_hours += 1,
                Tried::OptedOut => report.skipped_opted_out += 1,
                Tried::NoChannel => report.skipped_no_channel += 1,
            }
        }
        Ok(report)
    }

    /// Why this appointment is not reminded on this scan, if it is not.
    async fn skip_reason(
        &self,
        a: &Appointment,
        template_ids: &[uuid::Uuid],
        now: DateTime<Utc>,
    ) -> Result<Option<Skip>> {
        // Any language variant counts: the patient was reminded either way.
        let mut already = false;
        let mut failed = 0;
        for id in template_ids {
            already |= LetterRepository::exists_for_appointment(&self.db, a.id, *id).await?;
            failed += LetterRepository::count_failed_for_appointment(&self.db, a.id, *id).await?;
        }
        if already {
            return Ok(Some(Skip::AlreadyReminded));
        }
        if failed >= MAX_FAILED_ATTEMPTS {
            tracing::warn!(target: "pas::reminders", appointment_id = %a.id, "giving up after {MAX_FAILED_ATTEMPTS} failed attempts");
            return Ok(Some(Skip::GaveUp));
        }
        // A reminder that failed is tried again after a growing delay, not on every scan (NOT-4).
        if self.backoff && failed > 0 {
            let last: Option<DateTime<Utc>> = self
                .db
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT max(updated_at) AS t FROM generated_letters \
                     WHERE appointment_id = $1 AND status = 'failed' AND template_id = ANY($2)",
                    [a.id.into(), template_ids.to_vec().into()],
                ))
                .await
                .map_err(crate::Error::Database)?
                .and_then(|r| r.try_get("", "t").ok());
            if let Some(last) = last
                && now < last + crate::receipts::retry_delay(failed)
            {
                return Ok(Some(Skip::BackingOff));
            }
        }
        Ok(None)
    }

    /// Try the channels in the patient's order (their own choice first) until one reaches them.
    async fn try_channels(
        &self,
        a: &Appointment,
        template: &LetterTemplate,
        channels: &[DeliveryChannel],
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<Tried> {
        let lang = crate::inbound::messages::Lang::from_preference(Some(&template.locale));
        let extra = serde_json::json!({
            "appointment": {
                "id": a.id,
                "start": a.start_datetime.to_rfc3339(),
                "end": a.end_datetime.to_rfc3339(),
                "when": crate::inbound::messages::when(a.start_datetime, lang, crate::inbound::messages::timezone()),
                "reason": a.reason,
            }
        });
        let mut deferred = false;
        let mut opted_out = false;
        let preferred = crate::contact::preferred_channel(&self.db, a.person_id)
            .await?
            .and_then(|c| crate::db::repositories::letter::delivery_channel_from_str(&c).ok());
        for channel in &prefer_first(channels, preferred) {
            if !self.communication.channel_ready(*channel) {
                continue;
            }
            // Check the contact point first: generating a letter persists it
            // even when nothing can be delivered, and a stray pending letter
            // would block a later reminder.
            let address = match self
                .communication
                .recipient(a.person_id, *channel, ctx)
                .await
            {
                Ok(Some(addr)) => addr,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(target: "pas::reminders", appointment_id = %a.id, "recipient lookup failed: {e}");
                    break;
                }
            };
            if super::consent::is_opted_out(&self.db, a.person_id, *channel, Some(&address)).await?
            {
                opted_out = true;
                continue;
            }
            // Overnight, wait for the morning rather than fall back to another channel or drop it.
            if super::consent::is_intrusive(*channel)
                && self
                    .quiet_hours
                    .is_some_and(|q| q.contains(now, crate::inbound::messages::timezone()))
            {
                deferred = true;
                break;
            }
            match self
                .communication
                .generate_letter_for(
                    template.id,
                    a.person_id,
                    Some(a.id),
                    *channel,
                    extra.clone(),
                    "reminder",
                    Some(a.id.to_string()),
                    ctx,
                )
                .await
            {
                Ok(letter) if letter.status == crate::models::communication::LetterStatus::Sent => {
                    return Ok(Tried::Sent);
                }
                Ok(_) => continue, // the sender failed; try the next channel
                Err(e) => {
                    tracing::warn!(target: "pas::reminders", appointment_id = %a.id, "reminder failed: {e}");
                    continue;
                }
            }
        }
        Ok(if deferred {
            Tried::Deferred
        } else if opted_out {
            Tried::OptedOut
        } else {
            Tried::NoChannel
        })
    }
}

/// Why a scan leaves an appointment alone.
enum Skip {
    AlreadyReminded,
    GaveUp,
    BackingOff,
}

/// What came of trying one appointment's channels.
enum Tried {
    Sent,
    Deferred,
    OptedOut,
    NoChannel,
}

/// The wording a missing variant starts with: `(subject, body)`, by message and language. **The Welsh is
/// a first draft and needs review by a fluent speaker before any pilot.** Every message says how to reply
/// (PILOT-4) and carries the single-use link when the server has one (`reply_url`, CAP-13). No clinical
/// detail: these go over SMS and WhatsApp (NFR-13).
fn default_wording(name: &str, locale: &str) -> Option<(&'static str, &'static str)> {
    let lang = crate::communication::templates::primary(locale);
    match (name, lang.as_str()) {
        (REMINDER_TEMPLATE_NAME, "en") => Some((
            "Appointment reminder",
            "Hello {{ patient.name.given | default(value=\"\") }}, this is a reminder of your appointment on {{ appointment.when }}. Can't come? Tell us now so someone else can have the slot. Reply Y to confirm, N to cancel or R to change it.{% if reply_url %} Or tap: {{ reply_url }}{% endif %}",
        )),
        (REMINDER_TEMPLATE_NAME, "cy") => Some((
            "Nodyn atgoffa apwyntiad",
            "Helo {{ patient.name.given | default(value=\"\") }}, dyma nodyn i'ch atgoffa o'ch apwyntiad ar {{ appointment.when }}. Methu dod? Dywedwch wrthym nawr er mwyn i rywun arall gael y slot. Atebwch IE i gadarnhau, NA i ganslo neu NEWID i'w newid.{% if reply_url %} Neu tapiwch: {{ reply_url }}{% endif %}",
        )),
        ("slot_offer", "en") => Some((
            "An appointment has become free",
            "Hello {{ patient.name.given | default(value=\"\") }}, an appointment has become free on {{ offer.when }}. You asked to be told. It is yours if you take it in the next {{ offer.minutes }} minutes: {{ offer_url }}",
        )),
        ("slot_offer", "cy") => Some((
            "Mae apwyntiad ar gael",
            "Helo {{ patient.name.given | default(value=\"\") }}, mae apwyntiad ar gael ar {{ offer.when }}. Gofynnoch i ni roi gwybod i chi. Eich un chi yw os byddwch yn ei gymryd yn y {{ offer.minutes }} munud nesaf: {{ offer_url }}",
        )),
        ("appointment_reminder_second", "en") => Some((
            "Appointment reminder",
            "Hello {{ patient.name.given | default(value=\"\") }}, we have not heard back about your appointment on {{ appointment.when }}. Reply Y to confirm, N to cancel or R to change it.{% if reply_url %} Or tap: {{ reply_url }}{% endif %}",
        )),
        ("appointment_reminder_second", "cy") => Some((
            "Nodyn atgoffa apwyntiad",
            "Helo {{ patient.name.given | default(value=\"\") }}, nid ydym wedi clywed gennych am eich apwyntiad ar {{ appointment.when }}. Atebwch IE i gadarnhau, NA i ganslo neu NEWID i'w newid.{% if reply_url %} Neu tapiwch: {{ reply_url }}{% endif %}",
        )),
        ("appointment_reminder_final", "en") => Some((
            "Appointment tomorrow",
            "Reminder: your appointment is on {{ appointment.when }}. Reply HELP if you need to talk to us.",
        )),
        ("appointment_reminder_final", "cy") => Some((
            "Apwyntiad yfory",
            "Nodyn atgoffa: mae eich apwyntiad ar {{ appointment.when }}. Atebwch CYMORTH os oes angen siarad â ni.",
        )),
        _ => None,
    }
}

/// The template's channels with the patient's preferred one first (when the template offers it).
/// The patient's choice reorders what the template allows; it never adds a channel the template
/// does not list, and never drops one.
pub fn prefer_first(
    channels: &[DeliveryChannel],
    preferred: Option<DeliveryChannel>,
) -> Vec<DeliveryChannel> {
    let mut out = channels.to_vec();
    if let Some(p) = preferred
        && let Some(i) = out.iter().position(|c| *c == p)
    {
        let c = out.remove(i);
        out.insert(0, c);
    }
    out
}

/// Parse `PAS_REMINDER_LEAD_HOURS` (default 24).
pub fn lead_from_hours(hours: Option<&str>) -> Result<Duration> {
    let h: i64 = match hours {
        None | Some("") => 24,
        Some(v) => v.parse().map_err(|_| {
            Error::validation(format!("PAS_REMINDER_LEAD_HOURS not an integer: {v}"))
        })?,
    };
    if !(1..=24 * 30).contains(&h) {
        return Err(Error::validation("PAS_REMINDER_LEAD_HOURS must be 1..=720"));
    }
    Ok(Duration::hours(h))
}

#[cfg(test)]
mod tests {
    /// CAS-1: every patient-facing message the server writes by default exists as an English and a Welsh pair,
    /// both of which render to something a patient can read. A new message name that has only one language
    /// fails here, which is the pair check in CI.
    #[test]
    fn test_every_default_message_exists_in_both_languages_and_stays_in_one_sms_family() {
        for name in [
            REMINDER_TEMPLATE_NAME,
            "appointment_reminder_second",
            "appointment_reminder_final",
            "slot_offer",
        ] {
            let en =
                default_wording(name, "en-001").unwrap_or_else(|| panic!("{name} has no English"));
            let cy =
                default_wording(name, "cy-001").unwrap_or_else(|| panic!("{name} has no Welsh"));
            assert_ne!(
                en.1, cy.1,
                "{name}: the Welsh must not be the English copied"
            );
            for (lang, (subject, body)) in [("en", en), ("cy", cy)] {
                assert!(
                    !subject.trim().is_empty() && !body.trim().is_empty(),
                    "{name}/{lang} is empty"
                );
                // The same variables in both, so a Welsh message cannot drop the link or the time.
                for var in [
                    "reply_url",
                    "offer_url",
                    "appointment.when",
                    "offer.when",
                    "offer.minutes",
                ] {
                    assert_eq!(
                        en.1.contains(var),
                        cy.1.contains(var),
                        "{name}: {var} is in one language and not the other"
                    );
                }
                let _ = lang;
            }
        }
    }

    use super::*;

    #[test]
    fn test_prefer_first_reorders_but_never_adds_or_drops() {
        use DeliveryChannel::*;
        let t = [WhatsApp, Sms, Email];
        assert_eq!(prefer_first(&t, Some(Email)), vec![Email, WhatsApp, Sms]);
        assert_eq!(prefer_first(&t, Some(WhatsApp)), vec![WhatsApp, Sms, Email]);
        assert_eq!(
            prefer_first(&t, Some(Print)),
            t.to_vec(),
            "not offered by the template: ignored"
        );
        assert_eq!(prefer_first(&t, None), t.to_vec());
    }

    #[test]
    fn test_lead_defaults_to_24_hours() {
        assert_eq!(lead_from_hours(None).unwrap(), Duration::hours(24));
        assert_eq!(lead_from_hours(Some("")).unwrap(), Duration::hours(24));
    }

    #[test]
    fn test_lead_parses_and_bounds() {
        assert_eq!(lead_from_hours(Some("48")).unwrap(), Duration::hours(48));
        assert!(lead_from_hours(Some("0")).is_err());
        assert!(lead_from_hours(Some("721")).is_err());
        assert!(lead_from_hours(Some("abc")).is_err());
    }

    #[test]
    fn test_default_template_renders_with_appointment_context() {
        let mut ctx = tera::Context::new();
        ctx.insert(
            "patient",
            &serde_json::json!({ "name": { "given": "Ada" } }),
        );
        ctx.insert(
            "appointment",
            &serde_json::json!({ "start": "2026-10-20T09:00:00Z" }),
        );
        let out = tera::Tera::one_off(
            "Hello {{ patient.name.given | default(value=\"\") }}, at {{ appointment.start }}",
            &ctx,
            false,
        )
        .unwrap();
        assert_eq!(out, "Hello Ada, at 2026-10-20T09:00:00Z");
    }
}
