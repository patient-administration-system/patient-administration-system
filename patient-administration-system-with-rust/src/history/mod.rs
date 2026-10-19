//! Explaining a change or a contact (CAP-6).
//!
//! Two read-only views over what the PAS already records:
//!
//! - `appointment_history`: everything that happened to one appointment in time order, from the audit log
//!   (who, when, and a `reason_code` with its context), the messages sent about it (and what triggered
//!   each), the patient's replies and texts, and the reminder-ladder steps.
//! - `contact_log`: every message sent to one patient and every text received from them, newest first,
//!   with what triggered each one.
//!
//! Neither shows a message body or a text's content: they say that a contact happened and why, not what it
//! said. Both only read.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{Error, Result};

pub struct HistoryService {
    db: DatabaseConnection,
}

fn rows_to_json(rows: &[sea_orm::QueryResult]) -> Result<Vec<Value>> {
    rows.iter()
        .map(|r| {
            let at: chrono::DateTime<chrono::Utc> = r.try_get("", "at").map_err(Error::Database)?;
            let s = |c: &str| -> Result<Option<String>> {
                r.try_get::<Option<String>>("", c).map_err(Error::Database)
            };
            Ok(json!({
                "at": at,
                "kind": s("kind")?,
                "action": s("action")?,
                "by": s("by_user")?,
                "reason_code": s("reason_code")?,
                "context": r.try_get::<Option<Value>>("", "context").map_err(Error::Database)?,
            }))
        })
        .collect()
}

impl HistoryService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Everything that happened to one appointment, oldest first.
    pub async fn appointment_history(&self, appointment_id: Uuid) -> Result<Vec<Value>> {
        let exists = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT 1 AS ok FROM appointments WHERE id = $1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if exists.is_none() {
            return Err(Error::not_found(format!("appointment {appointment_id}")));
        }
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT 'audit' AS kind, at, action, user_id AS by_user, reason_code, context \
                   FROM audit_log WHERE entity_type = 'appointment' AND entity_id = $1 \
                 UNION ALL \
                 SELECT 'message', created_at, 'message_' || status, NULL, \"trigger\", \
                        jsonb_build_object('letter_id', id, 'channel', channel, 'locale', locale, 'trigger_ref', trigger_ref) \
                   FROM generated_letters WHERE appointment_id = $1 \
                 UNION ALL \
                 SELECT 'reply', received_at, 'reply_' || action || '_' || status, NULL, reject_reason, \
                        jsonb_build_object('channel', channel, 'message_id', message_id) \
                   FROM inbound_replies WHERE appointment_id = $1 \
                 UNION ALL \
                 SELECT 'text', received_at, 'text_' || outcome, NULL, intent, jsonb_build_object('carrier', carrier) \
                   FROM inbound_texts WHERE appointment_id = $1 \
                 UNION ALL \
                 SELECT 'reminder_step', created_at, 'reminder_step_' || step, NULL, outcome, \
                        jsonb_build_object('channel', channel, 'locale', locale, 'planned_for', planned_for) \
                   FROM reminder_steps WHERE appointment_id = $1 \
                 ORDER BY at",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows_to_json(&rows)
    }

    /// Every message sent to a patient and every text received from them, newest first.
    pub async fn contact_log(&self, person_id: Uuid, limit: i64) -> Result<Vec<Value>> {
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT * FROM ( \
                   SELECT 'message' AS kind, created_at AS at, 'message_' || status AS action, NULL::text AS by_user, \
                          \"trigger\" AS reason_code, \
                          jsonb_build_object('letter_id', id, 'channel', channel, 'locale', locale, \
                                             'trigger_ref', trigger_ref, 'appointment_id', appointment_id) AS context \
                     FROM generated_letters WHERE person_id = $1 \
                   UNION ALL \
                   SELECT 'text', received_at, 'text_' || outcome, NULL::text, intent, \
                          jsonb_build_object('appointment_id', appointment_id) \
                     FROM inbound_texts WHERE person_id = $1 \
                 ) c ORDER BY at DESC LIMIT $2",
                [person_id.into(), limit.clamp(1, 500).into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows_to_json(&rows)
    }
}
