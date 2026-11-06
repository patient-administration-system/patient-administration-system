//! Part of [`super`]: timed bed reservations (OFH-4).
//!
//! `pre_admit` reserves a bed. A reservation records the person, the planned encounter, who made it and,
//! when there is a hold time, when it lapses. `cancel_pre_admit` closes it as `cancelled`; the sweep closes a
//! lapsed one as `expired`, gives the bed back and cancels the planned encounter. Every change writes a row
//! to `bed_occupancy_events`. A hold with no `expires_at` never lapses, which is how it was before.
//!
//! **Not done:** `admit` still refuses a `Reserved` bed, so a reservation is closed only by a cancel or by
//! expiry, never by the admission it was for.

use crate::db::sql::{Conn, Stmt, Transact};
use chrono::{DateTime, Duration, Utc};
use sea_orm::DatabaseTransaction;

use super::*;
use crate::db::row::RowExt;

/// The most holds one sweep releases, so a long backlog is worked off over several passes.
const SWEEP_BATCH: i64 = 100;

/// One thing that happened to a bed's hold (OFH-4).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct OccupancyEvent {
    pub id: Uuid,
    pub bed_id: Uuid,
    /// `reserved`, `cancelled` or `expired`.
    pub event: String,
    #[schema(required = true)]
    pub reservation_id: Option<Uuid>,
    #[schema(required = true)]
    pub person_id: Option<Uuid>,
    /// The member of staff, or `system` for the sweep.
    #[schema(required = true)]
    pub actor: Option<String>,
    pub at: DateTime<Utc>,
}

impl AdtService {
    /// How long a hold lasts: the caller's `hold_minutes`, else `PAS_BED_HOLD_MINUTES`, else no expiry.
    pub(super) fn hold_for(hold_minutes: Option<i64>) -> Result<Option<Duration>> {
        let minutes = match hold_minutes {
            Some(m) if !(1..=10_080).contains(&m) => {
                return Err(Error::validation(
                    "hold_minutes must be between 1 and 10080 (seven days)",
                ));
            }
            Some(m) => m,
            None => crate::config::env::int("PAS_BED_HOLD_MINUTES"),
        };
        Ok((minutes > 0).then(|| Duration::minutes(minutes)))
    }

    /// Append one row to `bed_occupancy_events`. Always called in the same transaction as the change it
    /// records, so an event exists exactly when the change committed.
    async fn event(
        txn: &DatabaseTransaction,
        bed_id: Uuid,
        event: &str,
        reservation_id: Uuid,
        person_id: Uuid,
        actor: Option<&str>,
    ) -> Result<()> {
        txn.run(Stmt::new("INSERT INTO bed_occupancy_events (id, bed_id, event, reservation_id, person_id, actor) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            [
                Uuid::new_v4().into(),
                bed_id.into(),
                event.into(),
                reservation_id.into(),
                person_id.into(),
                actor.map(str::to_string).into(),
            ],
        ))
.await?;
        Ok(())
    }

    /// Record the hold that `pre_admit` has just made. The bed is already locked by the caller.
    pub(super) async fn open_reservation(
        txn: &DatabaseTransaction,
        bed_id: Uuid,
        person_id: Uuid,
        encounter_id: Uuid,
        expires_in: Option<Duration>,
        actor: Option<&str>,
    ) -> Result<()> {
        let id = Uuid::new_v4();
        let expires_at = expires_in.map(|d| Utc::now() + d);
        txn.run(Stmt::new("INSERT INTO bed_reservations (id, bed_id, person_id, encounter_id, held_by, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            [
                id.into(),
                bed_id.into(),
                person_id.into(),
                encounter_id.into(),
                actor.map(str::to_string).into(),
                expires_at.into(),
            ],
        ))
.await?;
        Self::event(txn, bed_id, "reserved", id, person_id, actor).await
    }

    /// Close the bed's open hold, if it has one: a bed reserved before OFH-4 has none, and cancelling its
    /// pre-admission still works.
    pub(super) async fn close_reservation(
        txn: &DatabaseTransaction,
        bed_id: Uuid,
        reason: &str,
        actor: Option<&str>,
    ) -> Result<()> {
        let closed = txn
            .one(Stmt::new(
                "UPDATE bed_reservations SET released_at = now(), release_reason = $2 \
                 WHERE bed_id = $1 AND released_at IS NULL RETURNING id, person_id",
                [bed_id.into(), reason.into()],
            ))
            .await?;
        if let Some(row) = closed {
            let (id, person): (Uuid, Uuid) = (row.col("id")?, row.col("person_id")?);
            Self::event(txn, bed_id, reason, id, person, actor).await?;
        }
        Ok(())
    }

    /// Give back every bed whose hold has lapsed at `now` (OFH-4): the bed goes `Reserved → Available`, the
    /// planned encounter is cancelled, the reservation closes as `expired`, and an audit row, an occupancy
    /// event and an outbox event are written, each hold in its own transaction. At most `SWEEP_BATCH` (100) a
    /// pass. Returns how many were released. A hold whose bed is no longer `Reserved` is closed without
    /// touching the bed.
    pub async fn release_expired_holds(&self, now: DateTime<Utc>) -> Result<u64> {
        let due = self
            .db
            .all(Stmt::new(
                "SELECT id, bed_id FROM bed_reservations \
                 WHERE released_at IS NULL AND expires_at IS NOT NULL AND expires_at <= $1 \
                 ORDER BY expires_at LIMIT $2",
                [now.into(), SWEEP_BATCH.into()],
            ))
            .await?;
        let mut released = 0;
        for row in due {
            let (id, bed_id): (Uuid, Uuid) = (row.col("id")?, row.col("bed_id")?);
            if self.release_one(id, bed_id, now).await? {
                released += 1;
            }
        }
        Ok(released)
    }

    /// Release one lapsed hold in a transaction of its own. Returns `false` when there was nothing to do.
    ///
    /// The order matters. The bed is locked **first**, as `admit`, `pre_admit` and `cancel_pre_admit` lock it,
    /// so a sweep and a cancel on the same bed queue behind each other and cannot deadlock. The reservation is
    /// then re-read under that lock with its conditions repeated (`released_at IS NULL`, `expires_at <= now`):
    /// the sweep chose it a moment ago, and a cancel may have closed it since. If so, this returns `false`
    /// and writes nothing.
    ///
    /// A bed that is no longer `Reserved` is left alone and only the reservation is closed, so the sweep never
    /// frees a bed that something else has since taken.
    async fn release_one(&self, id: Uuid, bed_id: Uuid, now: DateTime<Utc>) -> Result<bool> {
        let ctx = UserContext::system();
        // `Some((person, encounter))` when a hold was released; `None` when a cancel got there first.
        let done = self
            .db
            .in_txn::<_, Option<(Uuid, Uuid)>>(|txn| {
                Box::pin(async move {
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    // Re-read under the lock: a cancel may have closed it since the sweep looked.
                    let Some(r) = txn
                        .one(Stmt::new(
                            "SELECT person_id, encounter_id FROM bed_reservations \
                             WHERE id = $1 AND released_at IS NULL AND expires_at <= $2 FOR UPDATE",
                            [id.into(), now.into()],
                        ))
                        .await?
                    else {
                        return Ok(None);
                    };
                    let (person_id, encounter_id): (Uuid, Uuid) =
                        (r.col("person_id")?, r.col("encounter_id")?);
                    // Give the bed back only if it is still held. If it has been admitted to, taken out of service
                    // or cleaned since, it is not ours to change.
                    if bed.status == BedStatus::Reserved {
                        BedRepository::update_status(txn, bed_id, BedStatus::Available).await?;
                    }
                    // The planned encounter is cancelled only while it is still `Planned`: one that has
                    // already moved on is a real encounter and stays.
                    let enc = EncounterRepository::find_by_id(txn, encounter_id).await?;
                    if enc.is_some_and(|e| e.status == EncounterStatus::Planned) {
                        EncounterRepository::set_status(
                            txn,
                            encounter_id,
                            EncounterStatus::Cancelled,
                        )
                        .await?;
                    }
                    Self::close_reservation(txn, bed_id, "expired", Some("system")).await?;
                    AuditLogRepository::log(
                        txn,
                        "bed_reservation",
                        id,
                        "expired",
                        None,
                        Some(serde_json::json!({
                            "bed_id": bed_id, "person_id": person_id, "encounter_id": encounter_id,
                        })),
                        &ctx,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "BedHoldExpired",
                        &serde_json::json!({
                            "bed_id": bed_id, "person_id": person_id, "encounter_id": encounter_id,
                        }),
                    )
                    .await?;
                    Ok(Some((person_id, encounter_id)))
                })
            })
            .await?;
        let Some((person_id, encounter_id)) = done else {
            return Ok(false);
        };
        // The in-process event goes out only after the transaction has committed, and a failure to publish it
        // is logged, not raised: the outbox row written above is the durable record.
        self.publisher
            .publish(DomainEvent::new(
                "BedHoldExpired",
                serde_json::json!({
                    "bed_id": bed_id, "person_id": person_id, "encounter_id": encounter_id,
                }),
            ))
            .await
            .log_warn("in-process event");
        Ok(true)
    }

    /// What happened to a bed's hold, newest first. At most `fetch` rows (see `crate::paging`).
    pub async fn occupancy_events(&self, bed_id: Uuid, fetch: i64) -> Result<Vec<OccupancyEvent>> {
        let rows = self
            .db
            .all(Stmt::new("SELECT id, bed_id, event, reservation_id, person_id, actor, at FROM bed_occupancy_events \
                 WHERE bed_id = $1 ORDER BY at DESC, id LIMIT $2",
                [bed_id.into(), crate::paging::fetch(fetch).into()],
            ))
.await?;
        rows.iter()
            .map(|r| {
                Ok(OccupancyEvent {
                    id: r.col("id")?,
                    bed_id: r.col("bed_id")?,
                    event: r.col("event")?,
                    reservation_id: r.col("reservation_id")?,
                    person_id: r.col("person_id")?,
                    actor: r.col("actor")?,
                    at: r.col("at")?,
                })
            })
            .collect()
    }
}
