//! Consent, opt-out and quiet hours for outbound messages (PILOT-7, tasks.md NOT-5).
//!
//! The rules, written down (see also `governance/messaging-rules.md`):
//!
//! 1. **These are service messages** (appointment reminders, changes, replies to a patient): sent for
//!    the delivery of care, not marketing. PECR's consent rule for electronic marketing does not apply
//!    to them, but UK GDPR does, and a patient's wish to stop is always honoured.
//! 2. **STOP means stop.** A patient who texts STOP (or STOPIO) gets no more text-style messages (SMS,
//!    WhatsApp, Apple Messages), at that number, from this system, until they text START. It is held against the number as well as the
//!    person, so a STOP from a shared or unmatched number still works.
//! 3. **Quiet hours defer, never drop.** Text-style channels (SMS, WhatsApp, Apple Messages) are not used
//!    overnight (default 21:00 to 08:00 clinic time). A reminder due then waits for the morning; nothing
//!    is lost. Email and the portal are not intrusive and are not held back.
//! 4. **No marketing is sent by this system.** There is no marketing template category. If one is ever
//!    added it MUST require an active `Marketing` consent and PECR-compliant opt-in.

use crate::db::sql::{Conn, Stmt};
use jiff::{Timestamp, civil::Time};
use uuid::Uuid;

use crate::config::env;
use crate::db::row::RowExt;
use crate::models::communication::{
    DeliveryChannel, OptOutChannel, OptOutSource, OptionalKind, TemplateKind,
};
use crate::{Error, Result};

/// The default window: nothing intrusive between 9pm and 8am.
pub const DEFAULT_QUIET_HOURS: &str = "21:00-08:00";

/// A daily window in clinic time. It may cross midnight (`21:00-08:00`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietHours {
    start: Time,
    end: Time,
}

impl QuietHours {
    /// Parse `HH:MM-HH:MM`. Equal ends are refused (they would mean "never" or "always").
    pub fn parse(spec: &str) -> Result<Self> {
        let bad = || {
            Error::validation(format!(
                "PAS_QUIET_HOURS must look like 21:00-08:00, got {spec:?}"
            ))
        };
        let (a, b) = spec.trim().split_once('-').ok_or_else(bad)?;
        let start = a.trim().parse::<jiff::civil::Time>().map_err(|_| bad())?;
        let end = b.trim().parse::<jiff::civil::Time>().map_err(|_| bad())?;
        if start == end {
            return Err(bad());
        }
        Ok(Self { start, end })
    }

    /// From `PAS_QUIET_HOURS`: unset means the default, `off` means none.
    pub fn from_env() -> Result<Option<Self>> {
        match env::text("PAS_QUIET_HOURS") {
            Some(v) if v.trim().eq_ignore_ascii_case("off") => Ok(None),
            Some(v) => Self::parse(&v).map(Some),
            None => Self::parse(DEFAULT_QUIET_HOURS).map(Some),
        }
    }

    /// Is this instant inside the window, as the clinic's clock reads it?
    pub fn contains(&self, at: Timestamp, tz: jiff::tz::TimeZone) -> bool {
        let t = at.to_zoned_datetime(tz.clone()).time();
        if self.start < self.end {
            t >= self.start && t < self.end
        } else {
            t >= self.start || t < self.end
        }
    }
}

/// The text-style channels. A STOP or START covers all of them: "stop texting me" is not
/// specific to the app the word was typed in.
pub const TEXT_CHANNELS: [OptOutChannel; 3] = [
    OptOutChannel::Sms,
    OptOutChannel::WhatsApp,
    OptOutChannel::AppleMessages,
];

/// Channels held back during quiet hours.
pub fn is_intrusive(channel: DeliveryChannel) -> bool {
    matches!(
        channel,
        DeliveryChannel::Sms | DeliveryChannel::WhatsApp | DeliveryChannel::AppleMessages
    )
}

/// The opt-out channel name, or none for channels that cannot be opted out of one by one.
pub fn optout_channel(channel: DeliveryChannel) -> Option<OptOutChannel> {
    match channel {
        DeliveryChannel::Sms => Some(OptOutChannel::Sms),
        DeliveryChannel::WhatsApp => Some(OptOutChannel::WhatsApp),
        DeliveryChannel::AppleMessages => Some(OptOutChannel::AppleMessages),
        DeliveryChannel::Email => Some(OptOutChannel::Email),
        DeliveryChannel::Portal | DeliveryChannel::Print => None,
    }
}

pub fn person_subject(person: Uuid) -> String {
    format!("person:{person}")
}

/// Numbers are compared as stored (E.164); email addresses ignore case.
pub fn address_subject(address: &str) -> String {
    format!("address:{}", address.trim().to_lowercase())
}

/// Has the person, or the address, opted out of this channel?
pub async fn is_opted_out<C: Conn>(
    conn: &C,
    person: Uuid,
    channel: DeliveryChannel,
    address: Option<&str>,
) -> Result<bool> {
    let Some(ch) = optout_channel(channel) else {
        return Ok(false);
    };
    let mut subjects = vec![person_subject(person)];
    if let Some(a) = address {
        subjects.push(address_subject(a));
    }
    let row = conn
        .one(Stmt::new("SELECT 1 AS hit FROM message_optouts WHERE channel = $1 AND subject = ANY($2) AND resumed_at IS NULL LIMIT 1",
            [ch.as_str().into(), subjects.into()],
        ))
.await?;
    Ok(row.is_some())
}

/// Record an opt-out (idempotent; an earlier START is overturned).
pub async fn record_optout<C: Conn>(
    conn: &C,
    subject: &str,
    channel: OptOutChannel,
    source: OptOutSource,
    by: Option<&str>,
) -> Result<()> {
    conn.run(Stmt::new("INSERT INTO message_optouts (id, subject, channel, source, recorded_by) VALUES ($1,$2,$3,$4,$5) \
         ON CONFLICT (subject, channel) DO UPDATE SET opted_out_at = now(), resumed_at = NULL, source = EXCLUDED.source, \
           recorded_by = EXCLUDED.recorded_by",
        [Uuid::new_v4().into(), subject.into(), channel.as_str().into(), source.as_str().into(), by.map(String::from).into()],
    ))
.await?;
    Ok(())
}

/// Has the person, or the address, refused this kind of message (TSC-11)? A service message has no kind
/// and is never refused here.
pub async fn refuses_kind<C: Conn>(
    conn: &C,
    person: Uuid,
    address: Option<&str>,
    kind: OptionalKind,
) -> Result<bool> {
    let mut subjects = vec![person_subject(person)];
    if let Some(a) = address {
        subjects.push(address_subject(a));
    }
    let row = conn
        .one(Stmt::new(
            "SELECT 1 AS hit FROM message_kind_refusals WHERE kind = $1 AND subject = ANY($2) AND resumed_at IS NULL LIMIT 1",
            [kind.as_str().into(), subjects.into()],
        ))
        .await?;
    Ok(row.is_some())
}

/// Record that `subject` refuses a kind of message (idempotent; an earlier resume is overturned).
pub async fn refuse_kind<C: Conn>(
    conn: &C,
    subject: &str,
    kind: OptionalKind,
    source: OptOutSource,
    by: Option<&str>,
) -> Result<()> {
    conn.run(Stmt::new(
        "INSERT INTO message_kind_refusals (subject, kind, source, recorded_by) VALUES ($1,$2,$3,$4) \
         ON CONFLICT (subject, kind) DO UPDATE SET refused_at = now(), resumed_at = NULL, source = EXCLUDED.source, \
           recorded_by = EXCLUDED.recorded_by",
        [subject.into(), kind.as_str().into(), source.as_str().into(), by.map(String::from).into()],
    ))
    .await?;
    Ok(())
}

/// `subject` will take this kind of message again. Returns whether a refusal was active.
pub async fn resume_kind<C: Conn>(conn: &C, subject: &str, kind: OptionalKind) -> Result<bool> {
    let r = conn
        .run(Stmt::new(
            "UPDATE message_kind_refusals SET resumed_at = now() WHERE subject = $1 AND kind = $2 AND resumed_at IS NULL",
            [subject.into(), kind.as_str().into()],
        ))
        .await?;
    Ok(r.rows_affected() > 0)
}

/// The kind of the template group `template_id` belongs to, or none for a service message.
pub async fn template_kind<C: Conn>(conn: &C, template_id: Uuid) -> Result<Option<TemplateKind>> {
    let row = conn
        .one(Stmt::new(
            "SELECT k.kind FROM letter_templates t JOIN template_message_kinds k ON k.template_name = t.name WHERE t.id = $1",
            [template_id.into()],
        ))
        .await?;
    match row {
        None => Ok(None),
        Some(r) => Ok(Some(r.col::<String>("kind")?.parse().map_err(|_| {
            crate::Error::internal("template_message_kinds has an unknown kind")
        })?)),
    }
}

/// The patient asked to hear from us again. Returns whether anything was active.
pub async fn resume<C: Conn>(conn: &C, subject: &str, channel: OptOutChannel) -> Result<bool> {
    let r = conn
        .run(Stmt::new("UPDATE message_optouts SET resumed_at = now() WHERE subject = $1 AND channel = $2 AND resumed_at IS NULL",
            [subject.into(), channel.as_str().into()],
        ))
.await?;
    Ok(r.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(h: i8, m: i8) -> Timestamp {
        crate::time::utc(2026, 11, 3, h, m, 0) // November: London is GMT, same as UTC
    }
    fn london() -> jiff::tz::TimeZone {
        crate::time::zone(crate::config::env::DEFAULT_TIME_ZONE).unwrap()
    }

    #[test]
    fn test_a_window_that_crosses_midnight() {
        let q = QuietHours::parse("21:00-08:00").unwrap();
        for (h, m, inside) in [
            (20, 59, false),
            (21, 0, true),
            (23, 59, true),
            (0, 0, true),
            (7, 59, true),
            (8, 0, false),
            (12, 0, false),
        ] {
            assert_eq!(q.contains(utc(h, m), london()), inside, "{h}:{m:02}");
        }
    }

    #[test]
    fn test_a_window_inside_one_day() {
        let q = QuietHours::parse("13:00-14:00").unwrap();
        assert!(q.contains(utc(13, 30), london()));
        assert!(!q.contains(utc(14, 0), london()));
        assert!(!q.contains(utc(12, 59), london()));
    }

    #[test]
    fn test_the_clinic_clock_not_utc_decides() {
        // In July London is UTC+1: 20:30 UTC is 21:30 there, so quiet; 06:59 UTC is 07:59, still quiet.
        let q = QuietHours::parse("21:00-08:00").unwrap();
        let july = |h, m| crate::time::utc(2026, 7, 7, h, m, 0);
        assert!(q.contains(july(20, 30), london()));
        assert!(!q.contains(july(20, 30), jiff::tz::TimeZone::UTC));
        assert!(q.contains(july(6, 59), london()));
        assert!(!q.contains(july(7, 0), london()));
    }

    #[test]
    fn test_bad_windows_are_refused() {
        for bad in [
            "",
            "21:00",
            "21:00-21:00",
            "25:00-08:00",
            "21:00-8",
            "a-b",
            "21:00 to 08:00",
        ] {
            assert!(QuietHours::parse(bad).is_err(), "{bad:?}");
        }
        assert!(QuietHours::parse(" 21:00 - 08:00 ").is_ok());
    }

    #[test]
    fn test_only_text_style_channels_are_intrusive_and_only_some_can_be_opted_out() {
        use DeliveryChannel::*;
        assert!(is_intrusive(Sms) && is_intrusive(WhatsApp) && is_intrusive(AppleMessages));
        assert!(!is_intrusive(Email) && !is_intrusive(Portal) && !is_intrusive(Print));
        assert_eq!(optout_channel(Sms), Some(OptOutChannel::Sms));
        assert_eq!(optout_channel(Portal), None);
    }

    #[test]
    fn test_subjects_are_stable() {
        assert_eq!(address_subject(" A@B.Com "), "address:a@b.com");
        assert_eq!(address_subject("+447700900123"), "address:+447700900123");
    }
}
