//! Group consultations (NEW-18).
//!
//! A slot declares how many patients it takes (`capacity`): 1 is a one-to-one appointment, more is a group
//! session such as a virtual group clinic. Each attendee has their own appointment, so check-in, attendance,
//! no-show and follow-up are per attendee as for any appointment. The capacity is set before anyone books
//! and booking stops when it is reached: it is never overbooking.
//!
//! **The one-to-one option is always offered.** A group is something a patient or a member of staff
//! chooses. Nothing automatic puts anyone into one: rebook options, a referral's first available slot and
//! the standby backfill all take one-to-one slots only.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt, Transact, Txn};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::appointment::appointment_status_from_str;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::models::appointment::AppointmentStatus;
use crate::validation::named_person;
use crate::{Error, Result};

/// The largest group a slot may declare (also a database CHECK).
pub const MAX_CAPACITY: i32 = 50;

/// A slot's declared capacity and how many live appointments it holds. Read under the caller's lock.
pub async fn places<C: Conn>(conn: &C, slot_id: Uuid) -> Result<(i64, i64)> {
    let row = conn
        .one(Stmt::new("SELECT s.capacity::bigint AS capacity, \
               (SELECT count(*) FROM appointments a WHERE a.slot_id = s.id AND a.deleted_at IS NULL \
                  AND a.status NOT IN ('cancelled','no_show')) AS booked \
             FROM slots s WHERE s.id = $1",
            [slot_id.into()],
        ))
.await?
        .ok_or_else(|| Error::not_found(format!("slot {slot_id}")))?;
    Ok((row.col("capacity")?, row.col("booked")?))
}

/// Whether a slot is a group session (capacity above one).
pub async fn is_group(txn: &Txn, slot_id: Uuid) -> Result<bool> {
    Ok(places(txn, slot_id).await?.0 > 1)
}

#[derive(Debug, Serialize)]
pub struct Attendee {
    pub appointment_id: Uuid,
    pub person_id: Uuid,
    pub status: AppointmentStatus,
}

#[derive(Debug, Serialize)]
pub struct Session {
    pub slot_id: Uuid,
    pub capacity: i64,
    pub booked: i64,
    pub attendees: Vec<Attendee>,
}

pub struct GroupSessionService {
    db: Pool,
}

impl GroupSessionService {
    pub fn new(db: Pool) -> Self {
        Self { db }
    }

    /// Declare how many patients a future slot takes, by a named person; audited. Never below the number
    /// already booked.
    pub async fn set_capacity(
        &self,
        slot_id: Uuid,
        capacity: i32,
        set_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        self.set_capacity_if_unchanged(slot_id, capacity, set_by, None, ctx)
            .await
    }

    /// [`Self::set_capacity`] that refuses with `PreconditionFailed` when the slot has changed since the caller
    /// read it (OFH-11). `expected` `None` skips the comparison; the row is locked either way.
    pub async fn set_capacity_if_unchanged(
        &self,
        slot_id: Uuid,
        capacity: i32,
        set_by: &str,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        ctx: &UserContext,
    ) -> Result<()> {
        if !(1..=MAX_CAPACITY).contains(&capacity) {
            return Err(Error::validation(format!(
                "capacity must be between 1 and {MAX_CAPACITY}"
            )));
        }
        let by = named_person(
            set_by,
            "set_by: the person setting the capacity is required",
        )?;
        let txn = self.db.begin_txn().await?;
        let slot = txn
            .one(Stmt::new("SELECT capacity, updated_at, start_datetime > now() AS future FROM slots WHERE id = $1 FOR UPDATE",
                [slot_id.into()],
            ))
.await?
            .ok_or_else(|| Error::not_found(format!("slot {slot_id}")))?;
        let old: i32 = slot.col("capacity")?;
        let stored: chrono::DateTime<chrono::Utc> = slot.col("updated_at")?;
        if !crate::api::rest::concurrency::is_current(stored, expected) {
            return Err(Error::precondition_failed(
                "the slot has changed since it was read; read it again",
            ));
        }
        if !slot.col::<bool>("future")? {
            return Err(Error::conflict(
                "only a slot still to come can change its capacity",
            ));
        }
        let (_, booked) = places(&txn, slot_id).await?;
        if i64::from(capacity) < booked {
            return Err(Error::conflict(format!(
                "{booked} patients are booked: the capacity cannot be {capacity}"
            )));
        }
        // Free while a place is left, busy when full.
        txn.run(Stmt::new("UPDATE slots SET capacity = $2::integer, \
               status = CASE WHEN $3::bigint >= $2::integer THEN 'busy' WHEN status = 'busy' THEN 'free' ELSE status END, \
               updated_at = now() WHERE id = $1",
            [slot_id.into(), capacity.into(), booked.into()],
        ))
.await?;
        AuditLogRepository::log(
            &txn,
            "slot",
            slot_id,
            "set_capacity",
            Some(serde_json::json!({ "capacity": old })),
            Some(serde_json::json!({ "capacity": capacity, "set_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// A slot's capacity and its attendees (every appointment on it, with its own status).
    pub async fn session(&self, slot_id: Uuid) -> Result<Session> {
        let (capacity, booked) = places(&self.db, slot_id).await?;
        let rows = self
            .db
            .all(Stmt::new("SELECT id, person_id, status FROM appointments WHERE slot_id = $1 AND deleted_at IS NULL \
                 ORDER BY created_at, id",
                [slot_id.into()],
            ))
.await?;
        let attendees = rows
            .iter()
            .map(|r| {
                Ok(Attendee {
                    appointment_id: r.col("id")?,
                    person_id: r.col("person_id")?,
                    status: appointment_status_from_str(&r.col::<String>("status")?)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Session {
            slot_id,
            capacity,
            booked,
            attendees,
        })
    }
}
