//! Patient-led rebooking (CAP-5).
//!
//! The PAS offers a patient two or three real alternative slots for one appointment and the patient picks
//! one. The rules:
//!
//! - **Only real, safe slots are offered.** Same schedule (the same clinician or room) and the same length
//!   as the appointment, free now, not before the minimum notice, within the horizon, and never overlapping
//!   another appointment the patient has.
//! - **Spread, not clustered.** Three slots on one morning are not a choice. The offer prefers different
//!   days and different parts of the day, soonest first.
//! - **The offer is stored.** Choosing a slot that was not offered is refused, an offer can be used once,
//!   and it expires.
//! - **Nothing is lost on a refusal.** The old appointment is released as `rescheduled` and the new one
//!   booked in one transaction (`SchedulingService::rebook_tx`).
//! - It only ever adds a way to a better time: nothing here scores, ranks or limits a patient (NFR-11).

use crate::db::row::RowExt;
use std::sync::Arc;

use chrono::{DateTime, Duration, Timelike, Utc};
use chrono_tz::Tz;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::config::env;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::inbound::messages;
use crate::models::appointment::{Appointment, AppointmentStatus};
use crate::scheduling::SchedulingService;
use crate::{Error, Result};

/// How many alternatives to offer.
pub const OFFER_SIZE: usize = 3;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OfferedSlot {
    pub slot_id: Uuid,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Offer {
    /// `None` when there was nothing to offer; the caller then hands the patient to a person.
    pub offer_id: Option<Uuid>,
    pub appointment_id: Uuid,
    pub expires_at: Option<DateTime<Utc>>,
    pub time_zone: String,
    pub options: Vec<OfferedSlot>,
}

fn part_of_day(at: DateTime<Utc>, tz: Tz) -> u8 {
    match at.with_timezone(&tz).hour() {
        0..=11 => 0,
        12..=16 => 1,
        _ => 2,
    }
}

/// Choose up to `n` slots from `candidates` (sorted by start): prefer a new day and a new part of the day,
/// then a new day, then anything, always soonest first. The result is in time order.
pub fn spread(candidates: &[OfferedSlot], tz: Tz, n: usize) -> Vec<OfferedSlot> {
    let day = |s: &OfferedSlot| s.starts_at.with_timezone(&tz).date_naive();
    let mut chosen: Vec<OfferedSlot> = Vec::new();
    while chosen.len() < n {
        let fresh = |s: &&OfferedSlot| !chosen.iter().any(|c| c.slot_id == s.slot_id);
        let new_day = |s: &OfferedSlot| !chosen.iter().any(|c| day(c) == day(s));
        let new_part = |s: &OfferedSlot| {
            !chosen
                .iter()
                .any(|c| part_of_day(c.starts_at, tz) == part_of_day(s.starts_at, tz))
        };
        let pick = candidates
            .iter()
            .filter(fresh)
            .find(|s| new_day(s) && new_part(s))
            .or_else(|| candidates.iter().filter(fresh).find(|s| new_day(s)))
            .or_else(|| candidates.iter().find(|s| fresh(s)))
            .cloned();
        match pick {
            Some(p) => chosen.push(p),
            None => break,
        }
    }
    chosen.sort_by_key(|s| s.starts_at);
    chosen
}

pub struct RebookService {
    db: DatabaseConnection,
    scheduling: Arc<SchedulingService>,
}

impl RebookService {
    pub fn new(db: DatabaseConnection, scheduling: Arc<SchedulingService>) -> Self {
        Self { db, scheduling }
    }

    /// `PAS_REBOOK_HORIZON_DAYS` (default 42), how far ahead alternatives are looked for.
    fn horizon() -> Duration {
        Duration::days(env::int("PAS_REBOOK_HORIZON_DAYS"))
    }

    /// `PAS_REBOOK_MIN_NOTICE_HOURS` (default 24): a slot sooner than this is not offered.
    fn min_notice() -> Duration {
        Duration::hours(env::int("PAS_REBOOK_MIN_NOTICE_HOURS"))
    }

    /// `PAS_REBOOK_OFFER_MINUTES` (default 120): how long a patient has to choose.
    fn offer_ttl() -> Duration {
        Duration::minutes(env::int("PAS_REBOOK_OFFER_MINUTES"))
    }

    async fn movable(&self, appointment_id: Uuid) -> Result<Appointment> {
        let appt = AppointmentRepository::find_by_id(&self.db, appointment_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
        if !matches!(
            appt.status,
            AppointmentStatus::Booked | AppointmentStatus::Proposed
        ) {
            return Err(Error::invalid_transition(format!(
                "appointment {appointment_id} is {:?}; only a booked appointment can be moved",
                appt.status
            )));
        }
        if appt.start_datetime <= Utc::now() {
            return Err(Error::conflict("that appointment has already started"));
        }
        Ok(appt)
    }

    async fn candidates(&self, appt: &Appointment, now: DateTime<Utc>) -> Result<Vec<OfferedSlot>> {
        let Some(slot_id) = appt.slot_id else {
            return Ok(vec![]);
        };
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT s.id, s.start_datetime, s.end_datetime FROM slots s \
                 WHERE s.schedule_id = (SELECT schedule_id FROM slots WHERE id = $1) \
                   AND s.status = 'free' AND s.capacity = 1 AND NOT s.ring_fenced AND s.id <> $1 \
                   AND s.start_datetime >= $2 AND s.start_datetime < $3 \
                   AND (s.end_datetime - s.start_datetime) = ($4::timestamptz - $5::timestamptz) \
                   AND NOT EXISTS (SELECT 1 FROM appointments a \
                         WHERE a.person_id = $6 AND a.id <> $7 AND a.deleted_at IS NULL \
                           AND a.status NOT IN ('cancelled','no_show') \
                           AND a.start_datetime < s.end_datetime AND a.end_datetime > s.start_datetime) \
                 ORDER BY s.start_datetime LIMIT 300",
                [
                    slot_id.into(),
                    (now + Self::min_notice()).into(),
                    (now + Self::horizon()).into(),
                    appt.end_datetime.into(),
                    appt.start_datetime.into(),
                    appt.person_id.into(),
                    appt.id.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(OfferedSlot {
                    slot_id: r.col("id")?,
                    starts_at: r.col("start_datetime")?,
                    ends_at: r.col("end_datetime")?,
                })
            })
            .collect()
    }

    /// Two or three alternatives for one appointment. Asking again while an offer stands (and all its slots
    /// are still good) returns the same offer, so a refresh does not change the choices under the patient.
    pub async fn offer(&self, appointment_id: Uuid) -> Result<Offer> {
        let appt = self.movable(appointment_id).await?;
        let now = Utc::now();
        let tz = messages::timezone();
        let candidates = self.candidates(&appt, now).await?;
        let empty = || Offer {
            offer_id: None,
            appointment_id,
            expires_at: None,
            time_zone: tz.name().to_string(),
            options: vec![],
        };
        if candidates.is_empty() {
            return Ok(empty());
        }

        if let Some(row) = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, slot_ids, expires_at FROM rebook_offers \
                 WHERE appointment_id = $1 AND chosen_at IS NULL AND expires_at > now() \
                 ORDER BY created_at DESC LIMIT 1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?
        {
            let ids = stored_ids(&row)?;
            let still: Vec<OfferedSlot> = ids
                .iter()
                .filter_map(|id| candidates.iter().find(|c| c.slot_id == *id).cloned())
                .collect();
            if !ids.is_empty() && still.len() == ids.len() {
                return Ok(Offer {
                    offer_id: Some(row.col("id")?),
                    appointment_id,
                    expires_at: Some(row.col("expires_at")?),
                    time_zone: tz.name().to_string(),
                    options: still,
                });
            }
        }

        let options = spread(&candidates, tz, OFFER_SIZE);
        let offer_id = Uuid::new_v4();
        let expires_at = (now + Self::offer_ttl()).min(appt.start_datetime);
        let ids: Vec<String> = options.iter().map(|o| o.slot_id.to_string()).collect();
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO rebook_offers (id, appointment_id, person_id, slot_ids, expires_at) \
                 VALUES ($1,$2,$3,$4,$5)",
                [
                    offer_id.into(),
                    appointment_id.into(),
                    appt.person_id.into(),
                    serde_json::json!(ids).into(),
                    expires_at.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        Ok(Offer {
            offer_id: Some(offer_id),
            appointment_id,
            expires_at: Some(expires_at),
            time_zone: tz.name().to_string(),
            options,
        })
    }

    /// The offer standing for an appointment, if any, without making one. Used by the reply link to take
    /// the patient's choice.
    pub async fn standing_offer(&self, appointment_id: Uuid) -> Result<Option<Uuid>> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id FROM rebook_offers WHERE appointment_id = $1 AND chosen_at IS NULL \
                 AND expires_at > now() ORDER BY created_at DESC LIMIT 1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        row.map(|r| r.col("id")).transpose()
    }

    /// Take the patient's choice. Refused, changing nothing, if the offer is unknown, belongs to another
    /// appointment, was used, has expired, or does not contain the slot. Returns `(old, new)`.
    pub async fn choose(
        &self,
        appointment_id: Uuid,
        offer_id: Uuid,
        slot_id: Uuid,
        ctx: &UserContext,
    ) -> Result<(Appointment, Appointment)> {
        let ctx = ctx.clone();
        let (old, new) = self
            .db
            .transaction::<_, (Appointment, Appointment), Error>(|txn| {
                Box::pin(async move {
                    let row = txn
                        .query_one_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT appointment_id, slot_ids, expires_at, chosen_at IS NOT NULL AS used \
                             FROM rebook_offers WHERE id = $1 FOR UPDATE",
                            [offer_id.into()],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .ok_or_else(|| Error::validation("that offer does not exist"))?;
                    let for_appt: Uuid = row.col("appointment_id")?;
                    if for_appt != appointment_id {
                        return Err(Error::validation(
                            "that offer is for a different appointment",
                        ));
                    }
                    let used: bool = row.col("used")?;
                    if used {
                        return Err(Error::conflict("that offer has already been used"));
                    }
                    let expires: DateTime<Utc> =
                        row.col("expires_at")?;
                    if expires <= Utc::now() {
                        return Err(Error::validation("that offer has expired"));
                    }
                    if !stored_ids(&row)?.contains(&slot_id) {
                        return Err(Error::validation("that time was not one of the times offered"));
                    }
                    let (old, new) =
                        SchedulingService::rebook_tx(txn, appointment_id, slot_id, &ctx).await?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE rebook_offers SET chosen_slot_id = $2, chosen_at = now() WHERE id = $1",
                        [offer_id.into(), slot_id.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    AuditLogRepository::log(
                        txn,
                        "appointment",
                        old.id,
                        "rebook_offer_chosen",
                        None,
                        Some(serde_json::json!({
                            "offer_id": offer_id, "slot_id": slot_id, "new_appointment_id": new.id,
                        })),
                        &ctx,
                    )
                    .await?;
                    Ok((old, new))
                })
            })
            .await
            .map_err(Error::from)?;
        self.scheduling.publish_cancelled(&old).await;
        self.scheduling.publish_booked(&new).await;
        Ok((old, new))
    }
}

fn stored_ids(row: &sea_orm::QueryResult) -> Result<Vec<Uuid>> {
    let v: serde_json::Value = row.col("slot_ids")?;
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32, hour: u32) -> OfferedSlot {
        let t = Utc.with_ymd_and_hms(2030, 1, day, hour, 0, 0).unwrap();
        OfferedSlot {
            slot_id: Uuid::new_v4(),
            starts_at: t,
            ends_at: t + Duration::minutes(15),
        }
    }

    #[test]
    fn test_spread_prefers_other_days_and_parts_of_day() {
        // Plenty on Monday morning, then one Tuesday afternoon and one Wednesday evening.
        let c = vec![at(7, 9), at(7, 10), at(7, 11), at(8, 14), at(9, 18)];
        let got = spread(&c, chrono_tz::UTC, 3);
        assert_eq!(
            got.iter().map(|s| s.starts_at).collect::<Vec<_>>(),
            vec![c[0].starts_at, c[3].starts_at, c[4].starts_at]
        );
    }

    #[test]
    fn test_spread_falls_back_when_there_is_no_variety() {
        let c = vec![at(7, 9), at(7, 10), at(7, 11), at(7, 12)];
        let got = spread(&c, chrono_tz::UTC, 3);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].slot_id, c[0].slot_id, "soonest first");
        let mut ids: Vec<_> = got.iter().map(|s| s.slot_id).collect();
        ids.dedup();
        assert_eq!(ids.len(), 3, "no slot twice");
    }

    #[test]
    fn test_spread_with_fewer_candidates_than_asked() {
        assert!(spread(&[], chrono_tz::UTC, 3).is_empty());
        assert_eq!(spread(&[at(7, 9)], chrono_tz::UTC, 3).len(), 1);
    }

    #[test]
    fn test_day_and_part_use_the_clinic_zone_not_utc() {
        // 08:00 UTC is morning in London in January, but 03:00 in New York, still night = morning bucket;
        // 18:00 UTC is evening in London and 13:00 (afternoon) in New York.
        let s = at(7, 18);
        assert_eq!(part_of_day(s.starts_at, chrono_tz::Europe::London), 2);
        assert_eq!(part_of_day(s.starts_at, chrono_tz::America::New_York), 1);
        let m = at(7, 8);
        assert_eq!(part_of_day(m.starts_at, chrono_tz::Europe::London), 0);
    }
}
