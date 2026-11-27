//! A clinician's or a room's absence makes work, not cancellations (TSC-10).
//!
//! An absence is a time window on a practitioner or a room, with no reason and no detail: why someone is
//! away is not the PAS's business, only that they are. Recording one, in one transaction:
//!
//! - blocks the **free** slots of that owner's schedules inside the window (`blocked_out`), remembering
//!   which, so a withdrawal frees those and no others;
//! - lists the **booked** appointments it touches (on those slots, or with that practitioner as the appointment's \`worker_id\`), each of which
//!   becomes a work item (`absence_rebook`, TSC-1) for a person to rebook with the patient.
//!
//! Nothing here changes an appointment: none is cancelled or moved by itself, and the patient hears nothing
//! until a person acts (NFR-11). An item closes when its appointment is no longer booked in the window (the
//! person rebooked or cancelled it, in that feature's own place) or the absence is withdrawn.
//! `tests/tsc10_absences_test.rs` checks that recording and withdrawing change no appointment.

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt, Transact};
use crate::{Error, Result};

crate::string_enum! {
    /// Whose absence it is, as `staff_absences.owner_kind` (and `schedules.owner_kind`) store it.
    pub enum AbsentKind {
        Practitioner = "practitioner",
        Room = "room",
    }
}

/// The longest absence one record may cover.
pub const MAX_DAYS: i64 = 366;

/// An absence to record.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct NewAbsence {
    #[schema(value_type = String)]
    pub owner_kind: AbsentKind,
    pub owner_id: Uuid,
    pub starts_at: Timestamp,
    pub ends_at: Timestamp,
}

/// One absence, as the API sends it, with what it did.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Absence {
    pub id: Uuid,
    #[schema(value_type = String)]
    pub owner_kind: AbsentKind,
    pub owner_id: Uuid,
    pub starts_at: Timestamp,
    pub ends_at: Timestamp,
    pub recorded_by: String,
    pub recorded_at: Timestamp,
    #[schema(required = true)]
    pub withdrawn_at: Option<Timestamp>,
    /// The free slots it blocked.
    pub blocked_slots: Vec<Uuid>,
    /// The booked appointments it touched when it was recorded; each is a work item to rebook.
    pub appointments: Vec<Uuid>,
}

/// Check a window: it ends after it starts, and covers at most [`MAX_DAYS`].
pub fn check_window(starts_at: Timestamp, ends_at: Timestamp) -> Result<()> {
    if ends_at <= starts_at {
        return Err(Error::validation("an absence ends after it starts"));
    }
    if ends_at.duration_since(starts_at) > SignedDuration::from_hours(MAX_DAYS * 24) {
        return Err(Error::validation(format!(
            "an absence covers at most {MAX_DAYS} days; record a longer one in parts"
        )));
    }
    Ok(())
}

pub struct AbsenceService {
    db: Pool,
}

impl AbsenceService {
    pub fn new(db: Pool) -> Self {
        Self { db }
    }

    /// Record an absence: block the owner's free slots in the window and list the booked appointments it
    /// touches, in one transaction, audited. Then bring the work queue up to date so the items show at once.
    pub async fn record(&self, a: NewAbsence, who: &str, ctx: &UserContext) -> Result<Absence> {
        let who =
            crate::validation::named_person(who, "recording an absence needs a signed-in person")?;
        check_window(a.starts_at, a.ends_at)?;
        let id = Uuid::new_v4();
        let txn = self.db.begin_txn().await?;
        txn.run(Stmt::new(
            "INSERT INTO staff_absences (id, owner_kind, owner_id, starts_at, ends_at, recorded_by) \
             VALUES ($1,$2,$3,$4,$5,$6)",
            [
                id.into(),
                a.owner_kind.as_str().into(),
                a.owner_id.into(),
                a.starts_at.into(),
                a.ends_at.into(),
                who.into(),
            ],
        ))
        .await?;
        // The owner's free slots that overlap the window.
        let blocked: Vec<Uuid> = txn
            .all(Stmt::new(
                "UPDATE slots SET status = 'blocked_out', updated_at = now() \
                 WHERE status = 'free' AND start_datetime < $4 AND end_datetime > $3 \
                   AND schedule_id IN (SELECT id FROM schedules WHERE owner_kind = $1 AND owner_id = $2) \
                 RETURNING id",
                [
                    a.owner_kind.as_str().into(),
                    a.owner_id.into(),
                    a.starts_at.into(),
                    a.ends_at.into(),
                ],
            ))
            .await?
            .iter()
            .map(|r| r.col("id"))
            .collect::<Result<_>>()?;
        txn.run(Stmt::new(
            "INSERT INTO absence_blocked_slots (absence_id, slot_id) SELECT $1, unnest($2::uuid[])",
            [id.into(), blocked.clone().into()],
        ))
        .await?;
        // The booked appointments it touches: on one of the owner's slots, or with that practitioner.
        let appointments: Vec<Uuid> = txn
            .all(Stmt::new(
                "INSERT INTO absence_appointments (id, absence_id, appointment_id) \
                 SELECT gen_random_uuid(), $1, a.id FROM appointments a \
                 WHERE a.deleted_at IS NULL AND a.status = 'booked' \
                   AND a.start_datetime < $5 AND a.end_datetime > $4 \
                   AND (a.slot_id IN (SELECT s.id FROM slots s JOIN schedules h ON h.id = s.schedule_id \
                                       WHERE h.owner_kind = $2 AND h.owner_id = $3) \
                        OR ($2 = 'practitioner' AND a.worker_id = $3)) \
                 RETURNING appointment_id",
                [
                    id.into(),
                    a.owner_kind.as_str().into(),
                    a.owner_id.into(),
                    a.starts_at.into(),
                    a.ends_at.into(),
                ],
            ))
            .await?
            .iter()
            .map(|r| r.col("appointment_id"))
            .collect::<Result<_>>()?;
        AuditLogRepository::log(
            &txn,
            "staff_absence",
            id,
            "record",
            None,
            Some(serde_json::json!({
                "owner_kind": a.owner_kind.as_str(), "owner_id": a.owner_id,
                "starts_at": a.starts_at, "ends_at": a.ends_at,
                "blocked_slots": blocked.len(), "appointments": appointments.len(),
            })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        crate::work_queue::WorkQueueService::new(self.db.clone())
            .sync(Timestamp::now())
            .await?;
        self.get(id).await
    }

    /// Withdraw an absence: the slots it blocked that are still blocked, and no other absence also blocks,
    /// are free again, and its work items close. Audited.
    pub async fn withdraw(&self, id: Uuid, who: &str, ctx: &UserContext) -> Result<Absence> {
        let who = crate::validation::named_person(
            who,
            "withdrawing an absence needs a signed-in person",
        )?;
        let txn = self.db.begin_txn().await?;
        let done = txn
            .run(Stmt::new(
                "UPDATE staff_absences SET withdrawn_at = now(), withdrawn_by = $2 WHERE id = $1 AND withdrawn_at IS NULL",
                [id.into(), who.into()],
            ))
            .await?;
        if done.rows_affected() == 0 {
            // Either it does not exist or it was withdrawn already.
            self.get(id).await?;
            return Err(Error::conflict("this absence is already withdrawn"));
        }
        let freed = txn
            .run(Stmt::new(
                "UPDATE slots SET status = 'free', updated_at = now() \
                 WHERE status = 'blocked_out' \
                   AND id IN (SELECT slot_id FROM absence_blocked_slots WHERE absence_id = $1) \
                   AND NOT EXISTS (SELECT 1 FROM absence_blocked_slots b JOIN staff_absences s ON s.id = b.absence_id \
                                    WHERE b.slot_id = slots.id AND s.id <> $1 AND s.withdrawn_at IS NULL)",
                [id.into()],
            ))
            .await?
            .rows_affected();
        AuditLogRepository::log(
            &txn,
            "staff_absence",
            id,
            "withdraw",
            None,
            Some(serde_json::json!({ "freed_slots": freed })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        crate::work_queue::WorkQueueService::new(self.db.clone())
            .sync(Timestamp::now())
            .await?;
        self.get(id).await
    }

    /// One absence and what it did.
    pub async fn get(&self, id: Uuid) -> Result<Absence> {
        let r = self
            .db
            .one(Stmt::new(
                "SELECT * FROM staff_absences WHERE id = $1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("absence {id}")))?;
        let ids = |sql: &'static str, col: &'static str| {
            let db = self.db.clone();
            async move {
                db.all(Stmt::new(sql, [id.into()]))
                    .await?
                    .iter()
                    .map(|r| r.col::<Uuid>(col))
                    .collect::<Result<Vec<_>>>()
            }
        };
        Ok(Absence {
            id,
            owner_kind: r.col::<String>("owner_kind")?.parse()?,
            owner_id: r.col("owner_id")?,
            starts_at: r.col("starts_at")?,
            ends_at: r.col("ends_at")?,
            recorded_by: r.col("recorded_by")?,
            recorded_at: r.col("recorded_at")?,
            withdrawn_at: r.col("withdrawn_at")?,
            blocked_slots: ids(
                "SELECT slot_id FROM absence_blocked_slots WHERE absence_id = $1 ORDER BY slot_id",
                "slot_id",
            )
            .await?,
            appointments: ids(
                "SELECT appointment_id FROM absence_appointments WHERE absence_id = $1 ORDER BY appointment_id",
                "appointment_id",
            )
            .await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_ends_after_it_starts_and_covers_at_most_a_year() {
        let t = Timestamp::UNIX_EPOCH;
        assert!(check_window(t, t).is_err());
        assert!(check_window(t + SignedDuration::from_hours(1), t).is_err());
        assert!(check_window(t, t + SignedDuration::from_hours(MAX_DAYS * 24)).is_ok());
        assert!(check_window(t, t + SignedDuration::from_hours(MAX_DAYS * 24 + 1)).is_err());
    }
}
