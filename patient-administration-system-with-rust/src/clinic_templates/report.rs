//! Required against available capacity for one service (NEW-8).
//!
//! **This is not NHS England's core model.** That model is a spreadsheet whose formulas (percentile bands
//! over weekly demand, non-attendances split by whether the slot was reused or lost, patients removed before
//! booking, a sustainable waiting list size) are not published as text. This report uses plain arithmetic
//! that is stated here and in every response, so a reader can check it by hand:
//!
//! - **Demand per week**: referrals received for the service in a look-back window, divided by its weeks.
//! - **Non-attendance rate**: of appointments in that window that were attended or missed, the share missed.
//! - **Required slots per week**: demand per week divided by (1 − non-attendance rate). Each missed slot is
//!   treated as lost and the patient as needing another, which is the cautious reading; where slots are
//!   reused, the true need is lower.
//! - **Available slots per week**: slots on the service's active schedules in the forward window, divided by
//!   its weeks, with ring-fenced ones shown separately.
//!
//! It does not compute a sustainable waiting list size and does not smooth for seasonal variation. It
//! describes; it moves, removes and reorders nothing (NFR-11).

use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;

use crate::db::row::RowExt;
use crate::{Error, Result};

/// The longest forward window or look-back.
pub const MAX_DAYS: i64 = 366;

/// Slots needed per week for `demand_per_week` new demand when `non_attendance` of booked slots are missed.
/// With no rate (nothing to measure it from), or an impossible one, the demand itself.
pub fn required_per_week(demand_per_week: f64, non_attendance: Option<f64>) -> f64 {
    match non_attendance {
        Some(r) if (0.0..1.0).contains(&r) => demand_per_week / (1.0 - r),
        _ => demand_per_week,
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TypeRow {
    /// `new`, `follow_up`, `urgent`, `pifu`, or `untyped` for slots made without a template.
    pub slot_type: String,
    pub slots: i64,
    pub free: i64,
    pub ring_fenced: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CapacityReport {
    pub service_type: String,
    pub forward_from: DateTime<Utc>,
    pub forward_to: DateTime<Utc>,
    pub lookback_from: DateTime<Utc>,
    pub by_type: Vec<TypeRow>,
    pub available_slots: i64,
    pub ring_fenced_slots: i64,
    pub available_per_week: f64,
    pub referrals_in_lookback: i64,
    pub demand_per_week: f64,
    pub attended: i64,
    pub missed: i64,
    /// Missed over attended plus missed; `None` when there were none to measure.
    #[schema(required = true)]
    pub non_attendance_rate: Option<f64>,
    pub required_per_week: f64,
    /// Available minus required, per week. Negative means demand is not covered.
    pub gap_per_week: f64,
    pub method: &'static str,
    pub not_computed: &'static str,
}

pub async fn capacity_report(
    db: &DatabaseConnection,
    service_type: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    lookback_days: i64,
) -> Result<CapacityReport> {
    if service_type.trim().is_empty() {
        return Err(Error::validation("service_type is required"));
    }
    if to <= from || (to - from).num_days() > MAX_DAYS {
        return Err(Error::validation(format!(
            "the window must be forward and at most {MAX_DAYS} days"
        )));
    }
    if !(7..=MAX_DAYS).contains(&lookback_days) {
        return Err(Error::validation(format!(
            "lookback_days must be between 7 and {MAX_DAYS}"
        )));
    }
    let lookback_from = from - Duration::days(lookback_days);
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT coalesce(s.slot_type, 'untyped') AS slot_type, count(*) AS slots, \
                    count(*) FILTER (WHERE s.status = 'free') AS free, \
                    count(*) FILTER (WHERE s.ring_fenced) AS ring_fenced \
             FROM slots s JOIN schedules sc ON sc.id = s.schedule_id \
             WHERE sc.service_type = $1 AND sc.active AND s.status IN ('free','busy') \
               AND s.start_datetime >= $2 AND s.start_datetime < $3 \
             GROUP BY 1 ORDER BY 1",
            [service_type.into(), from.into(), to.into()],
        ))
        .await
        .map_err(Error::Database)?;
    let by_type = rows
        .iter()
        .map(|r| {
            Ok(TypeRow {
                slot_type: r.col("slot_type")?,
                slots: r.col("slots")?,
                free: r.col("free")?,
                ring_fenced: r.col("ring_fenced")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let available_slots: i64 = by_type.iter().map(|t| t.slots).sum();
    let ring_fenced_slots: i64 = by_type.iter().map(|t| t.ring_fenced).sum();

    let referrals: i64 = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM referrals WHERE target_service = $1 \
               AND received_at >= $2 AND received_at < $3",
            [service_type.into(), lookback_from.into(), from.into()],
        ))
        .await
        .map_err(Error::Database)?
        .ok_or_else(|| Error::internal("no row from count"))?
        .col("n")?;
    let outcomes = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) FILTER (WHERE a.status IN ('fulfilled','arrived')) AS attended, \
                    count(*) FILTER (WHERE a.status = 'no_show') AS missed \
             FROM appointments a JOIN slots s ON s.id = a.slot_id \
               JOIN schedules sc ON sc.id = s.schedule_id \
             WHERE sc.service_type = $1 AND a.deleted_at IS NULL \
               AND a.start_datetime >= $2 AND a.start_datetime < $3",
            [service_type.into(), lookback_from.into(), from.into()],
        ))
        .await
        .map_err(Error::Database)?
        .ok_or_else(|| Error::internal("no row from count"))?;
    let attended: i64 = outcomes.col("attended")?;
    let missed: i64 = outcomes.col("missed")?;
    let rate = (attended + missed > 0).then(|| missed as f64 / (attended + missed) as f64);

    let lookback_weeks = lookback_days as f64 / 7.0;
    let forward_weeks = (to - from).num_seconds() as f64 / (7.0 * 86_400.0);
    let demand_per_week = referrals as f64 / lookback_weeks;
    let required = required_per_week(demand_per_week, rate);
    let available_per_week = available_slots as f64 / forward_weeks;
    Ok(CapacityReport {
        service_type: service_type.to_string(),
        forward_from: from,
        forward_to: to,
        lookback_from,
        by_type,
        available_slots,
        ring_fenced_slots,
        available_per_week: round1(available_per_week),
        referrals_in_lookback: referrals,
        demand_per_week: round1(demand_per_week),
        attended,
        missed,
        non_attendance_rate: rate.map(|r| (r * 1000.0).round() / 1000.0),
        required_per_week: round1(required),
        gap_per_week: round1(available_per_week - required),
        method: "Plain arithmetic, not NHS England's core model: demand per week = referrals in the look-back \
                 / weeks; required = demand / (1 - non-attendance rate); available = slots in the window / \
                 weeks. A missed slot is treated as lost.",
        not_computed: "Sustainable waiting list size and seasonal variation are not computed.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_slots_allow_for_missed_appointments() {
        // 40 referrals a week, one booked slot in ten missed: 40 / 0.9 = 44.4 slots.
        assert!((required_per_week(40.0, Some(0.1)) - 44.444).abs() < 0.001);
        assert_eq!(required_per_week(40.0, Some(0.0)), 40.0);
    }

    #[test]
    fn with_no_usable_rate_the_demand_stands_alone() {
        assert_eq!(required_per_week(40.0, None), 40.0);
        assert_eq!(required_per_week(40.0, Some(1.0)), 40.0);
        assert_eq!(required_per_week(40.0, Some(-0.2)), 40.0);
    }
}
