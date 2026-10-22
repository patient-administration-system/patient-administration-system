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

use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait,
};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

/// The largest group a slot may declare (also a database CHECK).
pub const MAX_CAPACITY: i32 = 50;

/// A slot's declared capacity and how many live appointments it holds. Read under the caller's lock.
pub async fn places<C: ConnectionTrait>(conn: &C, slot_id: Uuid) -> Result<(i64, i64)> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT s.capacity::bigint AS capacity, \
               (SELECT count(*) FROM appointments a WHERE a.slot_id = s.id AND a.deleted_at IS NULL \
                  AND a.status NOT IN ('cancelled','no_show')) AS booked \
             FROM slots s WHERE s.id = $1",
            [slot_id.into()],
        ))
        .await
        .map_err(Error::Database)?
        .ok_or_else(|| Error::not_found(format!("slot {slot_id}")))?;
    Ok((
        row.try_get("", "capacity").map_err(Error::Database)?,
        row.try_get("", "booked").map_err(Error::Database)?,
    ))
}

/// Whether a slot is a group session (capacity above one).
pub async fn is_group(txn: &DatabaseTransaction, slot_id: Uuid) -> Result<bool> {
    Ok(places(txn, slot_id).await?.0 > 1)
}

#[derive(Debug, Serialize)]
pub struct Attendee {
    pub appointment_id: Uuid,
    pub person_id: Uuid,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct Session {
    pub slot_id: Uuid,
    pub capacity: i64,
    pub booked: i64,
    pub attendees: Vec<Attendee>,
}

pub struct GroupSessionService {
    db: DatabaseConnection,
}

impl GroupSessionService {
    pub fn new(db: DatabaseConnection) -> Self {
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
        if !(1..=MAX_CAPACITY).contains(&capacity) {
            return Err(Error::validation(format!(
                "capacity must be between 1 and {MAX_CAPACITY}"
            )));
        }
        let by = named_person(
            set_by,
            "set_by: the person setting the capacity is required",
        )?;
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let slot = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT capacity, start_datetime > now() AS future FROM slots WHERE id = $1 FOR UPDATE",
                [slot_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("slot {slot_id}")))?;
        let old: i32 = slot.try_get("", "capacity").map_err(Error::Database)?;
        if !slot
            .try_get::<bool>("", "future")
            .map_err(Error::Database)?
        {
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
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE slots SET capacity = $2::integer, \
               status = CASE WHEN $3::bigint >= $2::integer THEN 'busy' WHEN status = 'busy' THEN 'free' ELSE status END, \
               updated_at = now() WHERE id = $1",
            [slot_id.into(), capacity.into(), booked.into()],
        ))
        .await
        .map_err(Error::Database)?;
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
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, person_id, status FROM appointments WHERE slot_id = $1 AND deleted_at IS NULL \
                 ORDER BY created_at, id",
                [slot_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let attendees = rows
            .iter()
            .map(|r| {
                Ok(Attendee {
                    appointment_id: r.try_get("", "id").map_err(Error::Database)?,
                    person_id: r.try_get("", "person_id").map_err(Error::Database)?,
                    status: r.try_get("", "status").map_err(Error::Database)?,
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
