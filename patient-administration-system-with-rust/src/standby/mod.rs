//! Released slots and the standby list (CAP-3).
//!
//! When a future slot is freed by a cancellation or a miss, one `slot_releases` row records it and
//! a `SlotReleased` event is published. When anyone books that slot again the release is closed as
//! `refilled`, with the seconds it took. The booking team reads the releases to see which were
//! refilled and how fast, and reads the *candidates* for an open release to choose whom to offer it.
//!
//! The standby list holds patients who said they can come at short notice, with tags for language,
//! transport and phone-only contact. **Candidates are ordered by how long they have waited on
//! standby and by nothing else.** No score, flag or prediction takes part in who is offered a slot
//! (NFR-11): support can only ever add help, never decide who is passed over.
//!
//! Nothing here contacts anyone. Offering a slot is a person's decision until consent and opt-out
//! exist (NOT-5).

use chrono::{DateTime, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, FromQueryResult,
    Statement, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::models::appointment::{Appointment, ReleaseClass};
use crate::scheduling::SchedulingService;
use crate::{Error, Result};

fn db_err(e: sea_orm::DbErr) -> Error {
    Error::Database(e)
}

// --- the standby list -------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromQueryResult)]
pub struct StandbyEntry {
    pub id: Uuid,
    pub person_id: Uuid,
    pub service_type: String,
    pub short_notice_ok: bool,
    /// A locale code such as `cy-001`: the language the patient wants to be offered care in.
    pub language: Option<String>,
    pub needs_transport: bool,
    pub phone_only: bool,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StandbyInput {
    pub person_id: Uuid,
    pub service_type: String,
    #[serde(default = "yes")]
    pub short_notice_ok: bool,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub needs_transport: bool,
    #[serde(default)]
    pub phone_only: bool,
}

fn yes() -> bool {
    true
}

impl StandbyInput {
    pub fn validate(&self) -> Result<()> {
        let st = self.service_type.trim();
        if st.is_empty() || st.len() > 80 {
            return Err(Error::validation("service_type must be 1 to 80 characters"));
        }
        if let Some(l) = &self.language
            && !(2..=12).contains(&l.len())
        {
            return Err(Error::validation(
                "language must be a locale code such as cy-001",
            ));
        }
        Ok(())
    }
}

/// Optional tag filters a coordinator can apply to candidates.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CandidateFilter {
    pub language: Option<String>,
    pub needs_transport: Option<bool>,
    pub phone_only: Option<bool>,
}

// --- releases ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    Open,
    Refilled,
    /// Derived, never stored: still open after the slot's start time has passed.
    Expired,
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
pub struct SlotRelease {
    pub id: Uuid,
    pub slot_id: Uuid,
    pub appointment_id: Uuid,
    pub service_type: Option<String>,
    pub start_datetime: DateTime<Utc>,
    pub released_at: DateTime<Utc>,
    pub notice_days: Option<i32>,
    pub release_class: String,
    /// `open`, `refilled` or `expired` (derived).
    pub status: String,
    pub refilled_by_appointment_id: Option<Uuid>,
    pub refilled_at: Option<DateTime<Utc>>,
    pub seconds_to_refill: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
pub struct ReleaseSummary {
    pub total: i64,
    pub open: i64,
    pub refilled: i64,
    pub expired: i64,
    /// Median seconds from release to refill, over refilled releases. `None` when there are none.
    pub median_seconds_to_refill: Option<f64>,
}

const RELEASE_SELECT: &str = "SELECT r.id, r.slot_id, r.appointment_id, r.service_type, r.start_datetime, \
     r.released_at, r.notice_days, r.release_class, \
     CASE WHEN r.status = 'refilled' THEN 'refilled' WHEN r.start_datetime <= now() THEN 'expired' ELSE 'open' END AS status, \
     r.refilled_by_appointment_id, r.refilled_at, r.seconds_to_refill \
     FROM slot_releases r";

/// Record that a future slot was freed, and publish `SlotReleased`. A slot whose start has already
/// passed cannot be refilled, so it records nothing. Called inside the cancelling transaction.
pub(crate) async fn record_release(
    txn: &DatabaseTransaction,
    appt: &Appointment,
    class: ReleaseClass,
) -> Result<Option<Uuid>> {
    let Some(slot_id) = appt.slot_id else {
        return Ok(None);
    };
    if appt.start_datetime <= Utc::now() {
        return Ok(None);
    }
    let service: Option<String> = txn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT s.service_type FROM schedules s JOIN slots sl ON sl.schedule_id = s.id WHERE sl.id = $1",
            [slot_id.into()],
        ))
        .await
        .map_err(db_err)?
        .and_then(|r| r.try_get("", "service_type").ok());
    let id = Uuid::new_v4();
    let class_str = serde_json::to_value(class)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO slot_releases (id, slot_id, appointment_id, service_type, start_datetime, released_at, notice_days, release_class) \
         VALUES ($1,$2,$3,$4,$5,now(),$6,$7)",
        [
            id.into(),
            slot_id.into(),
            appt.id.into(),
            service.clone().into(),
            appt.start_datetime.into(),
            appt.notice_days.into(),
            class_str.clone().into(),
        ],
    ))
    .await
    .map_err(db_err)?;
    OutboxRepository::publish(
        txn,
        "SlotReleased",
        &serde_json::json!({
            "release_id": id,
            "slot_id": slot_id,
            "appointment_id": appt.id,
            "service_type": service,
            "start_datetime": appt.start_datetime,
            "notice_days": appt.notice_days,
            "release_class": class_str,
        }),
    )
    .await?;
    Ok(Some(id))
}

/// Close the open release for a slot that has just been booked. A booking of a slot with no open
/// release (the usual case) changes nothing.
pub(crate) async fn mark_refilled(
    txn: &DatabaseTransaction,
    slot_id: Uuid,
    by: &Appointment,
) -> Result<()> {
    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE slot_releases SET status = 'refilled', refilled_by_appointment_id = $2, refilled_at = now(), \
           seconds_to_refill = GREATEST(0, EXTRACT(EPOCH FROM (now() - released_at))::bigint) \
         WHERE slot_id = $1 AND status = 'open'",
        [slot_id.into(), by.id.into()],
    ))
    .await
    .map_err(db_err)?;
    Ok(())
}

// --- the service ------------------------------------------------------------------------------

pub struct StandbyService {
    db: DatabaseConnection,
    scheduling: Arc<SchedulingService>,
}

impl StandbyService {
    pub fn new(db: DatabaseConnection, scheduling: Arc<SchedulingService>) -> Self {
        Self { db, scheduling }
    }

    /// Put a patient on standby for a service, or update their tags if they already are.
    pub async fn add(&self, input: StandbyInput, ctx: &UserContext) -> Result<StandbyEntry> {
        input.validate()?;
        let ctx = ctx.clone();
        self.db
            .transaction::<_, StandbyEntry, Error>(|txn| {
                Box::pin(async move {
                    let entry = StandbyEntry::find_by_statement(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "INSERT INTO standby_entries (id, person_id, service_type, short_notice_ok, language, needs_transport, phone_only) \
                         VALUES ($1,$2,$3,$4,$5,$6,$7) \
                         ON CONFLICT (person_id, service_type) WHERE active \
                         DO UPDATE SET short_notice_ok = EXCLUDED.short_notice_ok, language = EXCLUDED.language, \
                           needs_transport = EXCLUDED.needs_transport, phone_only = EXCLUDED.phone_only, updated_at = now() \
                         RETURNING id, person_id, service_type, short_notice_ok, language, needs_transport, phone_only, active, created_at, updated_at",
                        [
                            Uuid::new_v4().into(),
                            input.person_id.into(),
                            input.service_type.trim().to_string().into(),
                            input.short_notice_ok.into(),
                            input.language.clone().into(),
                            input.needs_transport.into(),
                            input.phone_only.into(),
                        ],
                    ))
                    .one(txn)
                    .await
                    .map_err(db_err)?
                    .ok_or_else(|| Error::internal("standby upsert returned no row"))?;
                    AuditLogRepository::log(
                        txn,
                        "standby_entry",
                        entry.id,
                        "add",
                        None,
                        Some(serde_json::to_value(&entry).unwrap_or_default()),
                        &ctx,
                    )
                    .await?;
                    Ok(entry)
                })
            })
            .await
            .map_err(Error::from)
    }

    pub async fn list(&self, service_type: Option<&str>) -> Result<Vec<StandbyEntry>> {
        StandbyEntry::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT id, person_id, service_type, short_notice_ok, language, needs_transport, phone_only, active, created_at, updated_at \
             FROM standby_entries WHERE active AND ($1::text IS NULL OR service_type = $1) ORDER BY created_at, id",
            [service_type.map(String::from).into()],
        ))
        .all(&self.db)
        .await
        .map_err(db_err)
    }

    pub async fn deactivate(&self, id: Uuid, ctx: &UserContext) -> Result<()> {
        let ctx = ctx.clone();
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    let n = txn
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE standby_entries SET active = false, updated_at = now() WHERE id = $1 AND active",
                            [id.into()],
                        ))
                        .await
                        .map_err(db_err)?
                        .rows_affected();
                    if n == 0 {
                        return Err(Error::not_found(format!("standby entry {id}")));
                    }
                    AuditLogRepository::log(txn, "standby_entry", id, "deactivate", None, None, &ctx).await?;
                    Ok(())
                })
            })
            .await
            .map_err(Error::from)
    }

    /// Book a released (or any free) slot for a patient on standby, and take them off the list, in
    /// one transaction: either both happen or neither does.
    pub async fn book_from_standby(
        &self,
        id: Uuid,
        slot_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx = ctx.clone();
        let appt = self
            .db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let entry = StandbyEntry::find_by_statement(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "SELECT id, person_id, service_type, short_notice_ok, language, needs_transport, phone_only, active, created_at, updated_at \
                         FROM standby_entries WHERE id = $1 FOR UPDATE",
                        [id.into()],
                    ))
                    .one(txn)
                    .await
                    .map_err(db_err)?
                    .ok_or_else(|| Error::not_found(format!("standby entry {id}")))?;
                    if !entry.active {
                        return Err(Error::conflict("that standby entry is no longer active"));
                    }
                    let appt = SchedulingService::book_tx(txn, slot_id, entry.person_id, &ctx).await?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE standby_entries SET active = false, updated_at = now() WHERE id = $1",
                        [id.into()],
                    ))
                    .await
                    .map_err(db_err)?;
                    AuditLogRepository::log(
                        txn,
                        "standby_entry",
                        id,
                        "filled",
                        None,
                        Some(serde_json::json!({ "appointment_id": appt.id, "slot_id": slot_id })),
                        &ctx,
                    )
                    .await?;
                    Ok(appt)
                })
            })
            .await
            .map_err(Error::from)?;
        self.scheduling.publish_booked(&appt).await;
        Ok(appt)
    }

    pub async fn releases(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        status: Option<&str>,
    ) -> Result<(ReleaseSummary, Vec<SlotRelease>)> {
        if to <= from {
            return Err(Error::validation("`from` must precede `to`"));
        }
        if let Some(s) = status
            && !matches!(s, "open" | "refilled" | "expired")
        {
            return Err(Error::validation(
                "status must be open, refilled or expired",
            ));
        }
        let window = "r.released_at >= $1 AND r.released_at < $2";
        let rows = SlotRelease::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!("SELECT * FROM ({RELEASE_SELECT} WHERE {window}) x WHERE ($3::text IS NULL OR x.status = $3) ORDER BY x.released_at DESC"),
            [from.into(), to.into(), status.map(String::from).into()],
        ))
        .all(&self.db)
        .await
        .map_err(db_err)?;
        let summary = ReleaseSummary::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!(
                "SELECT count(*)::bigint AS total, \
                   count(*) FILTER (WHERE x.status = 'open')::bigint AS open, \
                   count(*) FILTER (WHERE x.status = 'refilled')::bigint AS refilled, \
                   count(*) FILTER (WHERE x.status = 'expired')::bigint AS expired, \
                   (percentile_cont(0.5) WITHIN GROUP (ORDER BY x.seconds_to_refill) FILTER (WHERE x.status = 'refilled'))::float8 AS median_seconds_to_refill \
                 FROM ({RELEASE_SELECT} WHERE {window}) x"
            ),
            [from.into(), to.into()],
        ))
        .one(&self.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| Error::internal("release summary returned no row"))?;
        Ok((summary, rows))
    }

    /// The standby patients who could take an open release, longest-waiting first. Excludes the
    /// patient who just gave the slot up, anyone inactive or not able to come at short notice, and
    /// anyone who already has an appointment overlapping the slot.
    pub async fn candidates(
        &self,
        release_id: Uuid,
        filter: &CandidateFilter,
    ) -> Result<Vec<StandbyEntry>> {
        let rel = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT r.service_type, r.appointment_id, r.status, sl.start_datetime, sl.end_datetime, a.person_id \
                 FROM slot_releases r JOIN slots sl ON sl.id = r.slot_id LEFT JOIN appointments a ON a.id = r.appointment_id \
                 WHERE r.id = $1",
                [release_id.into()],
            ))
            .await
            .map_err(db_err)?
            .ok_or_else(|| Error::not_found(format!("slot release {release_id}")))?;
        let status: String = rel.try_get("", "status").map_err(db_err)?;
        if status != "open" {
            return Err(Error::conflict("that slot has already been refilled"));
        }
        let service: Option<String> = rel.try_get("", "service_type").map_err(db_err)?;
        let Some(service) = service else {
            return Ok(Vec::new());
        };
        let start: DateTime<Utc> = rel.try_get("", "start_datetime").map_err(db_err)?;
        let end: DateTime<Utc> = rel.try_get("", "end_datetime").map_err(db_err)?;
        let gave_up: Option<Uuid> = rel.try_get("", "person_id").map_err(db_err)?;
        StandbyEntry::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT se.id, se.person_id, se.service_type, se.short_notice_ok, se.language, se.needs_transport, se.phone_only, se.active, se.created_at, se.updated_at \
             FROM standby_entries se \
             WHERE se.active AND se.short_notice_ok AND se.service_type = $1 \
               AND ($2::uuid IS NULL OR se.person_id <> $2) \
               AND NOT EXISTS (SELECT 1 FROM appointments ap WHERE ap.person_id = se.person_id AND ap.deleted_at IS NULL \
                               AND ap.status NOT IN ('cancelled','no_show') AND ap.start_datetime < $4 AND ap.end_datetime > $3) \
               AND ($5::text IS NULL OR se.language = $5) \
               AND ($6::boolean IS NULL OR se.needs_transport = $6) \
               AND ($7::boolean IS NULL OR se.phone_only = $7) \
             ORDER BY se.created_at ASC, se.id ASC",
            [
                service.into(),
                gave_up.into(),
                start.into(),
                end.into(),
                filter.language.clone().into(),
                filter.needs_transport.into(),
                filter.phone_only.into(),
            ],
        ))
        .all(&self.db)
        .await
        .map_err(db_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(service: &str) -> StandbyInput {
        StandbyInput {
            person_id: Uuid::new_v4(),
            service_type: service.into(),
            short_notice_ok: true,
            language: None,
            needs_transport: false,
            phone_only: false,
        }
    }

    #[test]
    fn test_validate_accepts_a_normal_entry_and_a_locale_code() {
        assert!(input("cardiology").validate().is_ok());
        let mut i = input("cardiology");
        i.language = Some("cy-001".into());
        assert!(i.validate().is_ok());
    }

    #[test]
    fn test_validate_refuses_an_empty_or_huge_service_and_a_nonsense_language() {
        assert!(input("   ").validate().is_err());
        assert!(input(&"x".repeat(81)).validate().is_err());
        let mut i = input("ent");
        i.language = Some("welsh please, thanks very much".into());
        assert!(i.validate().is_err());
    }

    #[test]
    fn test_input_defaults_to_short_notice_ok_with_no_tags() {
        let i: StandbyInput = serde_json::from_str(&format!(
            r#"{{"person_id":"{}","service_type":"ent"}}"#,
            Uuid::new_v4()
        ))
        .unwrap();
        assert!(i.short_notice_ok && !i.needs_transport && !i.phone_only && i.language.is_none());
    }
}
