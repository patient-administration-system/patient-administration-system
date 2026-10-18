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
use crate::models::communication::{DeliveryChannel, LetterTemplate};
use crate::{Error, Result};
use sea_orm::DatabaseConnection;

/// Name administrators edit to change the reminder wording.
pub const REMINDER_TEMPLATE_NAME: &str = "appointment_reminder";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReminderReport {
    pub considered: usize,
    pub sent: usize,
    pub skipped_already_reminded: usize,
    pub skipped_no_channel: usize,
}

pub struct ReminderService {
    db: DatabaseConnection,
    communication: Arc<CommunicationService>,
}

impl ReminderService {
    pub fn new(db: DatabaseConnection, communication: Arc<CommunicationService>) -> Self {
        Self { db, communication }
    }

    /// Insert the default template when no active one exists. Admins edit it
    /// afterwards via `PUT /api/letter-templates/{id}`.
    pub async fn ensure_default_template(&self) -> Result<LetterTemplate> {
        if let Some(t) =
            LetterRepository::find_active_template_by_name(&self.db, REMINDER_TEMPLATE_NAME).await?
        {
            return Ok(t);
        }
        let mut t = LetterTemplate::new(
            REMINDER_TEMPLATE_NAME.to_string(),
            "Appointment reminder".to_string(),
            "Hello {{ patient.name.given | default(value=\"\") }}, this is a reminder of your \
             appointment on {{ appointment.start }}. Please contact us if you cannot attend."
                .to_string(),
        );
        t.channels = vec![
            DeliveryChannel::WhatsApp,
            DeliveryChannel::AppleMessages,
            DeliveryChannel::Sms,
            DeliveryChannel::Email,
        ];
        LetterRepository::create_template(&self.db, &t).await
    }

    pub async fn run_once(&self, now: DateTime<Utc>, lead: Duration) -> Result<ReminderReport> {
        let template = self.ensure_default_template().await?;
        let appointments =
            AppointmentRepository::list_booked_starting_between(&self.db, now, now + lead).await?;
        let mut report = ReminderReport {
            considered: appointments.len(),
            ..Default::default()
        };
        let ctx = UserContext::system();
        for a in appointments {
            if LetterRepository::exists_for_appointment(&self.db, a.id, template.id).await? {
                report.skipped_already_reminded += 1;
                continue;
            }
            let extra = serde_json::json!({
                "appointment": {
                    "id": a.id,
                    "start": a.start_datetime.to_rfc3339(),
                    "end": a.end_datetime.to_rfc3339(),
                    "reason": a.reason,
                }
            });
            let mut sent = false;
            let preferred = crate::contact::preferred_channel(&self.db, a.person_id)
                .await?
                .and_then(|c| crate::db::repositories::letter::delivery_channel_from_str(&c).ok());
            for channel in &prefer_first(&template.channels, preferred) {
                if !self.communication.channel_ready(*channel) {
                    continue;
                }
                // Check the contact point first: generating a letter persists it
                // even when nothing can be delivered, and a stray pending letter
                // would block a later reminder.
                match self
                    .communication
                    .recipient(a.person_id, *channel, &ctx)
                    .await
                {
                    Ok(Some(_)) => {}
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::warn!(target: "pas::reminders", appointment_id = %a.id, "recipient lookup failed: {e}");
                        break;
                    }
                }
                match self
                    .communication
                    .generate_letter(
                        template.id,
                        a.person_id,
                        Some(a.id),
                        *channel,
                        extra.clone(),
                        &ctx,
                    )
                    .await
                {
                    Ok(letter)
                        if letter.status == crate::models::communication::LetterStatus::Sent =>
                    {
                        sent = true;
                        break;
                    }
                    Ok(_) => continue, // the sender failed; try the next channel
                    Err(e) => {
                        tracing::warn!(target: "pas::reminders", appointment_id = %a.id, "reminder failed: {e}");
                        continue;
                    }
                }
            }
            if sent {
                report.sent += 1;
            } else {
                report.skipped_no_channel += 1;
            }
        }
        Ok(report)
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
