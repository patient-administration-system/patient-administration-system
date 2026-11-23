//! Suspected conditions (WPAS): the stop reasons and target standings, the shapes the API sends, and the days
//! waited against a target. Recording, amending and reporting need the database and live in the server's
//! `suspicions` module, which re-exports this one.

use jiff::{Timestamp, civil::Date};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::waitlist::Priority;

pub mod profile;
pub use profile::Profile;

crate::string_enum! {
    /// Why a suspicion's clock stopped. Any one stops it, and only one is recorded.
    pub enum StopReason {
        /// The start of first definitive treatment.
        TreatmentStarted = "treatment_started",
        /// The suspicion was ruled out (for cancer, a downgrade).
        Downgraded = "downgraded",
        Died = "died",
        /// A clinician withdrew the suspicion: not ruled out, and not a mistake. Counted as stopped, and
        /// reported apart from the share within target until that is decided (WPAS-1a).
        Withdrawn = "withdrawn",
        /// Recorded by mistake: kept for the audit, left out of every count.
        EnteredInError = "entered_in_error",
    }
}

crate::string_enum! {
    /// Where a suspicion's wait stands against its target. Computed, never stored.
    pub enum TargetStatus {
        /// The condition has no target (FR-211).
        NoTarget = "no_target",
        Within = "within",
        /// Open, and within [`NEAR_TARGET_DAYS`] of the target: a display aid, not a rule.
        Near = "near",
        Past = "past",
    }
}

/// How close to a target an open wait is marked `near` (FR-212).
pub const NEAR_TARGET_DAYS: i64 = 7;

/// Today in the service's time zone, so "days waited" turns over at local midnight.
pub fn today() -> Date {
    Timestamp::now()
        .to_zoned_datetime(crate::inbound::messages::timezone())
        .date()
}

/// Whole days waited and where they stand against a target (FR-208, FR-211, FR-212).
pub fn wait(
    target: Option<i64>,
    suspected_on: Date,
    stopped_on: Option<Date>,
    today: Date,
) -> (i64, TargetStatus) {
    let days = crate::time::day_count(suspected_on, stopped_on.unwrap_or(today)).max(0);
    let status = match target {
        None => TargetStatus::NoTarget,
        Some(t) if days > t => TargetStatus::Past,
        Some(t)
            if stopped_on.is_none()
                && t.checked_sub(NEAR_TARGET_DAYS)
                    .is_some_and(|near| days > near) =>
        {
            TargetStatus::Near
        }
        Some(_) => TargetStatus::Within,
    };
    (days, status)
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Suspicion {
    pub id: Uuid,
    pub referral_id: Uuid,
    pub person_id: Uuid,
    /// The profile it was recorded under.
    pub profile: String,
    /// A condition code of that profile.
    pub condition: String,
    /// The condition's name in the active profile; none when it was recorded under another.
    #[schema(required = true)]
    pub condition_name: Option<String>,
    #[schema(required = true)]
    pub subtype: Option<String>,
    /// The list the subtype comes from.
    #[schema(required = true)]
    pub subtype_scheme: Option<String>,
    #[schema(required = true)]
    pub subtype_name: Option<String>,
    #[schema(required = true)]
    pub unique_pathway_identifier: Option<String>,
    /// The point of suspicion: the clock's start.
    pub suspected_on: Date,
    /// The clinician who suspected it.
    pub suspected_by: String,
    pub recorded_by: String,
    pub recorded_at: Timestamp,
    #[schema(required = true)]
    pub stop_reason: Option<StopReason>,
    #[schema(required = true)]
    pub stopped_on: Option<Date>,
    #[schema(required = true)]
    pub stopped_by: Option<String>,
    /// Whole days from the point of suspicion to the stop date, or to today while open. Never paused.
    pub days_waited: i64,
    /// The target recorded with it, or none (FR-211, FR-221).
    #[schema(required = true)]
    pub target_days: Option<i64>,
    pub target_status: TargetStatus,
    /// Whether a patient's own cancel is held for a person while it is open (FR-214).
    pub hold_patient_cancel: bool,
    /// The waiting-list priority it raises its referral to (FR-220).
    #[schema(required = true)]
    pub booking_priority: Option<Priority>,
    pub updated_at: Timestamp,
}

/// What a person records: the condition, the point of suspicion and who suspected it.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct NewSuspicion {
    /// A condition code of the active profile (`GET /api/suspicion-profile`).
    pub condition: String,
    /// A code from the condition's subtype list, if it has one; refused if it has none.
    #[serde(default)]
    pub subtype: Option<String>,
    /// The list the sender took `subtype` from. When given, it must be the active profile's list for the
    /// condition, so a code from another list is refused rather than relabelled (FR-223).
    #[serde(default)]
    pub subtype_scheme: Option<String>,
    #[serde(default)]
    pub unique_pathway_identifier: Option<String>,
    pub suspected_on: Date,
    pub suspected_by: String,
    pub recorded_by: String,
}

/// A correction to an open suspicion (FR-224). Each field left out is kept; at least one must change.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct Amendment {
    #[serde(default)]
    #[schema(required = false)]
    pub suspected_on: Option<Date>,
    #[serde(default)]
    #[schema(required = false)]
    pub suspected_by: Option<String>,
    /// A code from the condition's list in the profile it was recorded under.
    #[serde(default)]
    #[schema(required = false)]
    pub subtype: Option<String>,
    /// When given, must be that list's scheme.
    #[serde(default)]
    #[schema(required = false)]
    pub subtype_scheme: Option<String>,
    /// Why it is being corrected. Kept in the audit.
    pub reason: String,
    pub amended_by: String,
}

/// One row of the report (FR-213): suspicions whose clock stopped in the period.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReportRow {
    pub profile: String,
    pub condition: String,
    #[schema(required = true)]
    pub condition_name: Option<String>,
    #[schema(required = true)]
    pub subtype: Option<String>,
    #[schema(required = true)]
    pub subtype_scheme: Option<String>,
    /// The target they were recorded with; rows with different targets are counted apart.
    #[schema(required = true)]
    pub target_days: Option<i64>,
    /// Every stop in the period, withdrawn ones included.
    pub stopped: i64,
    /// Of `stopped`, those a clinician withdrew. They are left out of `within_target`, the share and the
    /// median until the sign-off decides how they count (WPAS-1a).
    pub withdrawn: i64,
    /// None when there is no target.
    #[schema(required = true)]
    pub within_target: Option<i64>,
    #[schema(required = true)]
    pub share_within_target: Option<f64>,
    #[schema(required = true)]
    pub median_days: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SuspicionReport {
    pub from: Date,
    pub to: Date,
    pub rows: Vec<ReportRow>,
}

/// The median of sorted whole days, or none for an empty list.
pub fn median(sorted: &[i64]) -> Option<f64> {
    let n = sorted.len();
    let at = |i: usize| sorted.get(i).copied();
    if n == 0 {
        None
    } else if n % 2 == 1 {
        at(n / 2).and_then(crate::arith::to_f64)
    } else {
        // Each half as a float first, so two large days cannot overflow an i64 sum.
        Some(
            (crate::arith::to_f64(at((n / 2).checked_sub(1)?)?)?
                + crate::arith::to_f64(at(n / 2)?)?)
                / 2.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    #[test]
    fn test_the_wait_is_real_days_and_marks_near_and_past() {
        let start = d("2026-01-01");
        let t = Some(62);
        assert_eq!(wait(t, start, None, d("2026-01-01")).0, 0);
        assert_eq!(
            wait(t, start, None, d("2026-02-25")).1,
            TargetStatus::Within,
            "day 55"
        );
        assert_eq!(
            wait(t, start, None, d("2026-02-26")).1,
            TargetStatus::Near,
            "day 56"
        );
        assert_eq!(
            wait(t, start, None, d("2026-03-04")).1,
            TargetStatus::Near,
            "day 62 is within, but near"
        );
        assert_eq!(
            wait(t, start, None, d("2026-03-05")).1,
            TargetStatus::Past,
            "day 63"
        );
        // A stopped clock is judged on its stop date, and is never "near".
        assert_eq!(
            wait(t, start, Some(d("2026-03-01")), d("2026-12-01")),
            (59, TargetStatus::Within)
        );
        assert_eq!(
            wait(t, start, Some(d("2026-03-10")), d("2026-12-01")).1,
            TargetStatus::Past
        );
        // No target: never past, however long.
        let none = wait(None, start, None, d("2027-06-01"));
        assert_eq!(none.1, TargetStatus::NoTarget);
        assert!(none.0 > 500);
    }

    #[test]
    fn test_median_of_odd_even_and_empty() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3]), Some(3.0));
        assert_eq!(median(&[1, 2, 10]), Some(2.0));
        assert_eq!(median(&[1, 2, 4, 10]), Some(3.0));
    }
}
