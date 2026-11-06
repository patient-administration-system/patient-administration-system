//! letter repository — LetterTemplate and GeneratedLetter

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::{generated_letter, letter_template};
use crate::models::communication::{
    DeliveryChannel, GeneratedLetter, LetterStatus, LetterTemplate,
};
use crate::{Error, Result};

pub struct LetterRepository;

impl LetterRepository {
    pub async fn create_template<C: Conn>(conn: &C, t: &LetterTemplate) -> Result<LetterTemplate> {
        let am = template_to_row(t)?;
        am.insert(conn).await?;
        Ok(t.clone())
    }

    pub async fn find_template_by_id<C: Conn>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<LetterTemplate>> {
        let m = conn
            .one_as::<letter_template::Model>(Stmt::new(
                "SELECT * FROM letter_templates WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(template_from_model).transpose()
    }

    pub async fn find_active_template_by_name<C: Conn>(
        conn: &C,
        name: &str,
    ) -> Result<Option<LetterTemplate>> {
        let m = conn
            .one_as::<letter_template::Model>(Stmt::new(
                "SELECT * FROM letter_templates WHERE name = $1 AND active = $2 LIMIT 1",
                [name.into(), true.into()],
            ))
            .await?;
        m.map(template_from_model).transpose()
    }

    /// Every active variant of a template group, English first by name of locale.
    pub async fn list_active_by_name<C: Conn>(conn: &C, name: &str) -> Result<Vec<LetterTemplate>> {
        let rows = conn
            .all_as::<letter_template::Model>(Stmt::new(
                "SELECT * FROM letter_templates WHERE name = $1 AND active = $2 ORDER BY locale",
                [name.into(), true.into()],
            ))
            .await?;
        rows.into_iter().map(template_from_model).collect()
    }

    /// Lock a template's row for the rest of the transaction and check it is the version the caller read
    /// (OFH-11). `expected` `None` (the caller sent `*`) skips the comparison but still takes the lock.
    pub async fn lock_template_if_current<C: Conn>(
        conn: &C,
        id: Uuid,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<LetterTemplate> {
        let m = conn
            .one_as::<letter_template::Model>(Stmt::new(
                "SELECT * FROM letter_templates WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("letter template {id}")))?;
        let t = template_from_model(m)?;
        if !crate::api::rest::concurrency::is_current(t.updated_at, expected) {
            return Err(Error::precondition_failed(
                "the letter template has changed since it was read; read it again",
            ));
        }
        Ok(t)
    }

    /// Replace the editable fields of a template (administrator edit).
    pub async fn update_template<C: Conn>(conn: &C, t: &LetterTemplate) -> Result<LetterTemplate> {
        let existing = conn
            .one_as::<letter_template::Model>(Stmt::new(
                "SELECT * FROM letter_templates WHERE id = $1 LIMIT 1",
                [t.id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("letter template {}", t.id)))?;
        let mut row = template_to_row(t)?;
        row.created_at = existing.created_at;
        row.updated_at = chrono::Utc::now().fixed_offset();
        let updated = row.update_row(conn).await?;
        template_from_model(updated)
    }

    /// True when a non-failed reminder letter already exists for the appointment.
    pub async fn exists_for_appointment<C: Conn>(
        conn: &C,
        appointment_id: Uuid,
        template_id: Uuid,
    ) -> Result<bool> {
        let n = conn.count(Stmt::new("SELECT count(*) AS n FROM generated_letters WHERE appointment_id = $1 AND template_id = $2 AND status <> $3", [appointment_id.into(), template_id.into(), "failed".into()])).await?;
        Ok(n > 0)
    }

    /// A person's letters, newest first, optionally only one channel.
    pub async fn list_for_person<C: Conn>(
        conn: &C,
        person_id: Uuid,
        channel: Option<DeliveryChannel>,
    ) -> Result<Vec<GeneratedLetter>> {
        let rows = conn
            .all_as::<generated_letter::Model>(match channel {
                Some(c) => Stmt::new(
                    "SELECT * FROM generated_letters WHERE person_id = $1 AND channel = $2 ORDER BY created_at DESC",
                    [person_id.into(), delivery_channel_to_str(c).into()],
                ),
                None => Stmt::new(
                    "SELECT * FROM generated_letters WHERE person_id = $1 ORDER BY created_at DESC",
                    [person_id.into()],
                ),
            })
            .await?;
        rows.into_iter().map(letter_from_model).collect()
    }

    /// How many delivery attempts for this appointment and template have failed.
    pub async fn count_failed_for_appointment<C: Conn>(
        conn: &C,
        appointment_id: Uuid,
        template_id: Uuid,
    ) -> Result<i64> {
        let n = conn.count(Stmt::new("SELECT count(*) AS n FROM generated_letters WHERE appointment_id = $1 AND template_id = $2 AND status = $3", [appointment_id.into(), template_id.into(), "failed".into()])).await?;
        Ok(i64::try_from(n).unwrap_or(i64::MAX))
    }

    pub async fn list_active_templates<C: Conn>(conn: &C) -> Result<Vec<LetterTemplate>> {
        let rows = conn
            .all_as::<letter_template::Model>(Stmt::new(
                "SELECT * FROM letter_templates WHERE active = $1",
                [true.into()],
            ))
            .await?;
        rows.into_iter().map(template_from_model).collect()
    }

    pub async fn create_generated_letter<C: Conn>(
        conn: &C,
        g: &GeneratedLetter,
    ) -> Result<GeneratedLetter> {
        let am = letter_to_row(g);
        am.insert(conn).await?;
        Ok(g.clone())
    }

    pub async fn find_generated_letter_by_id<C: Conn>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<GeneratedLetter>> {
        let m = conn
            .one_as::<generated_letter::Model>(Stmt::new(
                "SELECT * FROM generated_letters WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(letter_from_model).transpose()
    }

    pub async fn mark_sent<C: Conn>(conn: &C, id: Uuid) -> Result<GeneratedLetter> {
        let m = conn
            .one_as::<generated_letter::Model>(Stmt::new(
                "SELECT * FROM generated_letters WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("generated_letter {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set(
            "status",
            letter_status_to_str(LetterStatus::Sent).to_string(),
        );
        am.set("sent_at", Some(now));
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        letter_from_model(updated)
    }

    pub async fn mark_failed<C: Conn>(conn: &C, id: Uuid) -> Result<GeneratedLetter> {
        let m = conn
            .one_as::<generated_letter::Model>(Stmt::new(
                "SELECT * FROM generated_letters WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("generated_letter {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set(
            "status",
            letter_status_to_str(LetterStatus::Failed).to_string(),
        );
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        letter_from_model(updated)
    }
}

// --- conversion helpers ---

pub(crate) fn delivery_channel_to_str(c: DeliveryChannel) -> &'static str {
    match c {
        DeliveryChannel::Print => "print",
        DeliveryChannel::Email => "email",
        DeliveryChannel::Sms => "sms",
        DeliveryChannel::AppleMessages => "apple_messages",
        DeliveryChannel::WhatsApp => "whatsapp",
        DeliveryChannel::Portal => "portal",
    }
}

pub(crate) fn delivery_channel_from_str(s: &str) -> Result<DeliveryChannel> {
    match s {
        "print" => Ok(DeliveryChannel::Print),
        "email" => Ok(DeliveryChannel::Email),
        "sms" => Ok(DeliveryChannel::Sms),
        "apple_messages" => Ok(DeliveryChannel::AppleMessages),
        "whatsapp" => Ok(DeliveryChannel::WhatsApp),
        "portal" => Ok(DeliveryChannel::Portal),
        other => Err(Error::internal(format!(
            "unknown delivery channel: {other}"
        ))),
    }
}

pub(crate) fn letter_status_to_str(s: LetterStatus) -> &'static str {
    match s {
        LetterStatus::Pending => "pending",
        LetterStatus::Sent => "sent",
        LetterStatus::Failed => "failed",
    }
}

pub(crate) fn letter_status_from_str(s: &str) -> Result<LetterStatus> {
    match s {
        "pending" => Ok(LetterStatus::Pending),
        "sent" => Ok(LetterStatus::Sent),
        "failed" => Ok(LetterStatus::Failed),
        other => Err(Error::internal(format!("unknown letter status: {other}"))),
    }
}

fn template_to_row(t: &LetterTemplate) -> Result<letter_template::Model> {
    let required = serde_json::to_value(&t.required_variables)
        .map_err(|e| Error::internal(format!("serialize required_variables: {e}")))?;
    let channels = serde_json::to_value(&t.channels)
        .map_err(|e| Error::internal(format!("serialize channels: {e}")))?;
    Ok(letter_template::Model {
        id: t.id,
        name: t.name.clone(),
        subject: t.subject.clone(),
        body_tera: t.body_tera.clone(),
        required_variables: required,
        channels,
        active: t.active,
        locale: t.locale.clone(),
        created_at: t.created_at.fixed_offset(),
        updated_at: t.updated_at.fixed_offset(),
    })
}

fn template_from_model(m: letter_template::Model) -> Result<LetterTemplate> {
    Ok(LetterTemplate {
        id: m.id,
        name: m.name,
        subject: m.subject,
        body_tera: m.body_tera,
        required_variables: serde_json::from_value(m.required_variables)
            .map_err(|e| Error::internal(format!("deserialize required_variables: {e}")))?,
        channels: serde_json::from_value(m.channels)
            .map_err(|e| Error::internal(format!("deserialize channels: {e}")))?,
        active: m.active,
        locale: m.locale,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn letter_to_row(g: &GeneratedLetter) -> generated_letter::Model {
    generated_letter::Model {
        id: g.id,
        template_id: g.template_id,
        person_id: g.person_id,
        appointment_id: g.appointment_id,
        rendered_subject: g.rendered_subject.clone(),
        rendered_body: g.rendered_body.clone(),
        channel: delivery_channel_to_str(g.channel).to_string(),
        status: letter_status_to_str(g.status).to_string(),
        sent_at: g.sent_at.map(|t| t.fixed_offset()),
        locale: g.locale.clone(),
        trigger: g.trigger.clone(),
        trigger_ref: g.trigger_ref.clone(),
        created_at: g.created_at.fixed_offset(),
        updated_at: g.updated_at.fixed_offset(),
    }
}

fn letter_from_model(m: generated_letter::Model) -> Result<GeneratedLetter> {
    Ok(GeneratedLetter {
        id: m.id,
        template_id: m.template_id,
        person_id: m.person_id,
        appointment_id: m.appointment_id,
        rendered_subject: m.rendered_subject,
        rendered_body: m.rendered_body,
        channel: delivery_channel_from_str(&m.channel)?,
        status: letter_status_from_str(&m.status)?,
        sent_at: m.sent_at.map(|t| t.with_timezone(&chrono::Utc)),
        locale: m.locale,
        trigger: m.trigger,
        trigger_ref: m.trigger_ref,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_letter_template_roundtrip_via_row() {
        let mut t = LetterTemplate::new(
            "appointment_reminder".into(),
            "Your appointment".into(),
            "Hello {{ name }}".into(),
        );
        t.required_variables = vec!["name".into()];
        t.channels = vec![DeliveryChannel::Email, DeliveryChannel::Print];
        let am = template_to_row(&t).expect("to_row");
        let m = letter_template::Model {
            id: am.id,
            name: am.name.clone(),
            subject: am.subject.clone(),
            body_tera: am.body_tera.clone(),
            required_variables: am.required_variables.clone(),
            channels: am.channels.clone(),
            active: am.active,
            locale: am.locale.clone(),
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = template_from_model(m).expect("from_model");
        assert_eq!(back.id, t.id);
        assert_eq!(back.required_variables, vec!["name".to_string()]);
        assert_eq!(back.channels.len(), 2);
        assert!(back.active);
    }
}
