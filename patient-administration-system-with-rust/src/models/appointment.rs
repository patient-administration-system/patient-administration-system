//! appointment

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result};

/// The lifecycle status of an appointment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppointmentStatus {
    Proposed,
    Booked,
    Arrived,
    Fulfilled,
    Cancelled,
    NoShow,
}

impl AppointmentStatus {
    /// Attempt to transition from `self` to `next`. Returns `Err` if the
    /// transition is not allowed (including same-state transitions and
    /// transitions from terminal states).
    pub fn try_transition_to(self, next: AppointmentStatus) -> Result<AppointmentStatus> {
        if self == next {
            return Err(Error::invalid_transition(format!(
                "AppointmentStatus already {:?}",
                self
            )));
        }
        let ok = matches!(
            (self, next),
            (AppointmentStatus::Proposed, AppointmentStatus::Booked)
                | (AppointmentStatus::Proposed, AppointmentStatus::Cancelled)
                | (AppointmentStatus::Booked, AppointmentStatus::Arrived)
                | (AppointmentStatus::Booked, AppointmentStatus::Cancelled)
                | (AppointmentStatus::Booked, AppointmentStatus::NoShow)
                | (AppointmentStatus::Arrived, AppointmentStatus::Fulfilled)
                | (AppointmentStatus::Arrived, AppointmentStatus::Cancelled)
        );
        if ok {
            Ok(next)
        } else {
            Err(Error::invalid_transition(format!(
                "Cannot transition AppointmentStatus from {:?} to {:?}",
                self, next
            )))
        }
    }
}

/// Reason an appointment was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationReason {
    PatientRequest,
    ProviderRequest,
    NoShow,
    Rescheduled,
    Other,
}

/// How the appointment is held (CAP-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    #[default]
    InPerson,
    Video,
    Phone,
}

impl Modality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InPerson => "in_person",
            Self::Video => "video",
            Self::Phone => "phone",
        }
    }
}

/// Where a conversion to a video or phone consultation stands (CAP-14). A clinician can veto before
/// the slot is confirmed as virtual; a veto restores `in_person`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionState {
    PendingReview,
    Confirmed,
    Vetoed,
}

/// Who ended an appointment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelledBy {
    Patient,
    Provider,
    System,
}

impl CancelledBy {
    /// The coarse legacy reason this maps to.
    pub fn legacy_reason(self) -> CancellationReason {
        match self {
            CancelledBy::Patient => CancellationReason::PatientRequest,
            CancelledBy::Provider => CancellationReason::ProviderRequest,
            CancelledBy::System => CancellationReason::Other,
        }
    }
}

/// Why a patient could not or did not attend: the barrier, in the patient's own terms.
/// These feed the discovery work on access barriers, so they describe the situation, not blame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarrierCode {
    CannotTravel,
    Caring,
    Work,
    FeelBetter,
    Unwell,
    Forgot,
    Other,
    /// Anxiety or fear about the appointment or the place.
    Anxiety,
    Cost,
    /// The patient no longer needs the appointment (CAP-8: ask "still need this?" before closing).
    NoLongerNeeded,
    /// Could not cancel, or did not know how to.
    CouldNotCancel,
    /// A communication or access need was not met (language, hearing, sight, mobility).
    AccessNeed,
}

impl BarrierCode {
    /// Every code, in the order shown to a patient. The database CHECK must list exactly these.
    pub const ALL: [BarrierCode; 12] = [
        Self::CannotTravel,
        Self::Caring,
        Self::Work,
        Self::FeelBetter,
        Self::Unwell,
        Self::Forgot,
        Self::Anxiety,
        Self::Cost,
        Self::NoLongerNeeded,
        Self::CouldNotCancel,
        Self::AccessNeed,
        Self::Other,
    ];

    /// The wire and database form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CannotTravel => "cannot_travel",
            Self::Caring => "caring",
            Self::Work => "work",
            Self::FeelBetter => "feel_better",
            Self::Unwell => "unwell",
            Self::Forgot => "forgot",
            Self::Other => "other",
            Self::Anxiety => "anxiety",
            Self::Cost => "cost",
            Self::NoLongerNeeded => "no_longer_needed",
            Self::CouldNotCancel => "could_not_cancel",
            Self::AccessNeed => "access_need",
        }
    }
}

/// How an ended appointment relates to the chance to refill its slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseClass {
    /// Cancelled with at least [`EARLY_RELEASE_DAYS`] days' notice: the slot can be refilled.
    EarlyRelease,
    /// Cancelled with less notice: refilling is difficult.
    LateRelease,
    /// A no-show: nobody told us, and the slot was lost.
    SilentMiss,
}

/// Notice, in whole days, at or above which a cancellation counts as an early release.
pub const EARLY_RELEASE_DAYS: i32 = 3;

/// Whole days between `cancelled_at` and the start, never negative (cancelling after the start
/// gives no notice).
pub fn notice_days(cancelled_at: DateTime<Utc>, start: DateTime<Utc>) -> i32 {
    let days = (start - cancelled_at).num_days();
    days.clamp(0, i64::from(i32::MAX)) as i32
}

/// Classify an ended appointment. `None` for one that has not ended without attendance.
pub fn classify_release(
    status: AppointmentStatus,
    notice_days: Option<i32>,
) -> Option<ReleaseClass> {
    match status {
        AppointmentStatus::NoShow => Some(ReleaseClass::SilentMiss),
        AppointmentStatus::Cancelled => Some(match notice_days {
            Some(d) if d >= EARLY_RELEASE_DAYS => ReleaseClass::EarlyRelease,
            _ => ReleaseClass::LateRelease,
        }),
        _ => None,
    }
}

/// An appointment scheduled for a patient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Appointment {
    pub id: Uuid,
    pub person_id: Uuid,
    pub slot_id: Option<Uuid>,
    pub worker_id: Option<Uuid>,
    pub start_datetime: DateTime<Utc>,
    pub end_datetime: DateTime<Utc>,
    pub status: AppointmentStatus,
    pub reason: Option<String>,
    pub from_waitlist_entry_id: Option<Uuid>,
    pub cancellation_reason: Option<CancellationReason>,
    /// Backlink to an [`crate::models::appointment_series::AppointmentSeries`]
    /// when this appointment was generated as one occurrence of a
    /// recurring series (v0.9). `None` for singleton appointments.
    #[serde(default)]
    pub series_id: Option<Uuid>,
    /// When it was cancelled (CAP-1).
    #[serde(default)]
    pub cancelled_at: Option<DateTime<Utc>>,
    /// Who cancelled it (CAP-1).
    #[serde(default)]
    pub cancelled_by: Option<CancelledBy>,
    /// The patient's barrier, when known (CAP-1). Can be recorded after a no-show too.
    #[serde(default)]
    pub barrier_code: Option<BarrierCode>,
    /// Whole days of notice given (CAP-1).
    #[serde(default)]
    pub notice_days: Option<i32>,
    /// When the patient confirmed they are coming (CAP-2).
    #[serde(default)]
    pub confirmed_at: Option<DateTime<Utc>>,
    /// How it is held (CAP-14). `in_person` unless converted.
    #[serde(default)]
    pub modality: Modality,
    /// Set once a patient has asked for a video or phone consultation (CAP-14).
    #[serde(default)]
    pub conversion_state: Option<ConversionState>,
    /// The barrier behind a conversion request (CAP-14), apart from any cancellation barrier.
    #[serde(default)]
    pub conversion_barrier_code: Option<BarrierCode>,
    #[serde(default)]
    pub converted_at: Option<DateTime<Utc>>,
    /// Derived from status and notice; never stored, so it cannot disagree with them.
    #[serde(default)]
    pub release_class: Option<ReleaseClass>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Appointment {
    pub fn new(
        person_id: Uuid,
        start_datetime: DateTime<Utc>,
        end_datetime: DateTime<Utc>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            person_id,
            slot_id: None,
            worker_id: None,
            start_datetime,
            end_datetime,
            status: AppointmentStatus::Proposed,
            reason: None,
            from_waitlist_entry_id: None,
            cancellation_reason: None,
            series_id: None,
            cancelled_at: None,
            cancelled_by: None,
            barrier_code: None,
            notice_days: None,
            confirmed_at: None,
            modality: Modality::InPerson,
            conversion_state: None,
            conversion_barrier_code: None,
            converted_at: None,
            release_class: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_appointment_new_defaults() {
        let person_id = Uuid::new_v4();
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let appt = Appointment::new(person_id, start, end);
        assert_eq!(appt.person_id, person_id);
        assert!(appt.slot_id.is_none());
        assert!(appt.worker_id.is_none());
        assert_eq!(appt.start_datetime, start);
        assert_eq!(appt.end_datetime, end);
        assert_eq!(appt.status, AppointmentStatus::Proposed);
        assert!(appt.reason.is_none());
        assert!(appt.from_waitlist_entry_id.is_none());
        assert!(appt.cancellation_reason.is_none());
        assert_eq!(appt.created_at, appt.updated_at);
    }

    #[test]
    fn test_appointment_status_proposed_to_booked_ok() {
        let result = AppointmentStatus::Proposed.try_transition_to(AppointmentStatus::Booked);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), AppointmentStatus::Booked);
    }

    #[test]
    fn test_appointment_status_booked_to_fulfilled_fails() {
        // Must go through Arrived first.
        let result = AppointmentStatus::Booked.try_transition_to(AppointmentStatus::Fulfilled);
        assert!(result.is_err());
    }

    #[test]
    fn test_appointment_status_cancelled_is_terminal() {
        for next in [
            AppointmentStatus::Proposed,
            AppointmentStatus::Booked,
            AppointmentStatus::Arrived,
            AppointmentStatus::Fulfilled,
            AppointmentStatus::Cancelled,
            AppointmentStatus::NoShow,
        ] {
            let result = AppointmentStatus::Cancelled.try_transition_to(next);
            assert!(
                result.is_err(),
                "Cancelled should not transition to {:?}",
                next
            );
        }
    }

    #[test]
    fn test_appointment_status_fulfilled_is_terminal() {
        for next in [
            AppointmentStatus::Proposed,
            AppointmentStatus::Booked,
            AppointmentStatus::Arrived,
            AppointmentStatus::Fulfilled,
            AppointmentStatus::Cancelled,
            AppointmentStatus::NoShow,
        ] {
            let result = AppointmentStatus::Fulfilled.try_transition_to(next);
            assert!(
                result.is_err(),
                "Fulfilled should not transition to {:?}",
                next
            );
        }
    }

    #[test]
    fn test_appointment_status_no_show_is_terminal() {
        for next in [
            AppointmentStatus::Proposed,
            AppointmentStatus::Booked,
            AppointmentStatus::Arrived,
            AppointmentStatus::Fulfilled,
            AppointmentStatus::Cancelled,
            AppointmentStatus::NoShow,
        ] {
            let result = AppointmentStatus::NoShow.try_transition_to(next);
            assert!(
                result.is_err(),
                "NoShow should not transition to {:?}",
                next
            );
        }
    }

    #[test]
    fn test_appointment_status_same_state_fails() {
        assert!(
            AppointmentStatus::Proposed
                .try_transition_to(AppointmentStatus::Proposed)
                .is_err()
        );
        assert!(
            AppointmentStatus::Booked
                .try_transition_to(AppointmentStatus::Booked)
                .is_err()
        );
    }

    #[test]
    fn test_appointment_status_booked_transitions() {
        assert!(
            AppointmentStatus::Booked
                .try_transition_to(AppointmentStatus::Arrived)
                .is_ok()
        );
        assert!(
            AppointmentStatus::Booked
                .try_transition_to(AppointmentStatus::Cancelled)
                .is_ok()
        );
        assert!(
            AppointmentStatus::Booked
                .try_transition_to(AppointmentStatus::NoShow)
                .is_ok()
        );
    }

    #[test]
    fn test_appointment_status_arrived_transitions() {
        assert!(
            AppointmentStatus::Arrived
                .try_transition_to(AppointmentStatus::Fulfilled)
                .is_ok()
        );
        assert!(
            AppointmentStatus::Arrived
                .try_transition_to(AppointmentStatus::Cancelled)
                .is_ok()
        );
    }

    #[test]
    fn test_appointment_serde_roundtrip() {
        let person_id = Uuid::new_v4();
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let mut appt = Appointment::new(person_id, start, end);
        appt.reason = Some("Routine checkup".to_string());
        appt.cancellation_reason = Some(CancellationReason::PatientRequest);
        appt.worker_id = Some(Uuid::new_v4());
        appt.slot_id = Some(Uuid::new_v4());
        appt.from_waitlist_entry_id = Some(Uuid::new_v4());

        let json = serde_json::to_string(&appt).expect("serialize");
        let back: Appointment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, appt.id);
        assert_eq!(back.person_id, appt.person_id);
        assert_eq!(back.slot_id, appt.slot_id);
        assert_eq!(back.worker_id, appt.worker_id);
        assert_eq!(back.status, appt.status);
        assert_eq!(back.reason, appt.reason);
        assert_eq!(back.from_waitlist_entry_id, appt.from_waitlist_entry_id);
        assert_eq!(back.cancellation_reason, appt.cancellation_reason);
        // Verify snake_case rename on CancellationReason.
        assert!(json.contains("patient_request"));
        // Verify snake_case rename on AppointmentStatus.
        assert!(json.contains("proposed"));
    }

    #[test]
    fn test_cancellation_reason_serde_all_variants() {
        for r in [
            CancellationReason::PatientRequest,
            CancellationReason::ProviderRequest,
            CancellationReason::NoShow,
            CancellationReason::Rescheduled,
            CancellationReason::Other,
        ] {
            let json = serde_json::to_string(&r).expect("serialize");
            let back: CancellationReason = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, r);
        }
        assert_eq!(
            serde_json::to_string(&CancellationReason::PatientRequest).unwrap(),
            "\"patient_request\""
        );
        assert_eq!(
            serde_json::to_string(&CancellationReason::NoShow).unwrap(),
            "\"no_show\""
        );
    }

    #[test]
    fn test_notice_days_counts_whole_days_and_never_goes_negative() {
        let start = Utc::now() + chrono::Duration::days(10);
        assert_eq!(notice_days(start - chrono::Duration::days(5), start), 5);
        assert_eq!(
            notice_days(start - chrono::Duration::hours(47), start),
            1,
            "47h is one whole day"
        );
        assert_eq!(notice_days(start - chrono::Duration::hours(2), start), 0);
        assert_eq!(
            notice_days(start + chrono::Duration::hours(1), start),
            0,
            "after the start: no notice"
        );
    }

    #[test]
    fn test_release_class_separates_early_late_and_silent() {
        use AppointmentStatus::*;
        assert_eq!(
            classify_release(Cancelled, Some(EARLY_RELEASE_DAYS)),
            Some(ReleaseClass::EarlyRelease)
        );
        assert_eq!(
            classify_release(Cancelled, Some(10)),
            Some(ReleaseClass::EarlyRelease)
        );
        assert_eq!(
            classify_release(Cancelled, Some(EARLY_RELEASE_DAYS - 1)),
            Some(ReleaseClass::LateRelease)
        );
        assert_eq!(
            classify_release(Cancelled, Some(0)),
            Some(ReleaseClass::LateRelease)
        );
        assert_eq!(
            classify_release(Cancelled, None),
            Some(ReleaseClass::LateRelease),
            "unknown notice is not assumed early"
        );
        assert_eq!(
            classify_release(NoShow, None),
            Some(ReleaseClass::SilentMiss)
        );
        assert_eq!(
            classify_release(NoShow, Some(9)),
            Some(ReleaseClass::SilentMiss),
            "a no-show is silent whatever the notice"
        );
        assert_eq!(classify_release(Booked, None), None);
        assert_eq!(classify_release(Fulfilled, None), None);
    }

    #[test]
    fn test_barrier_codes_use_the_agreed_wire_names() {
        let names: Vec<String> = BarrierCode::ALL
            .iter()
            .map(|b| {
                serde_json::to_value(b)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            [
                "cannot_travel",
                "caring",
                "work",
                "feel_better",
                "unwell",
                "forgot",
                "anxiety",
                "cost",
                "no_longer_needed",
                "could_not_cancel",
                "access_need",
                "other"
            ]
        );
        // `as_str` (used for the database) and serde (used on the wire) must agree, and every code
        // must round-trip.
        for b in BarrierCode::ALL {
            assert_eq!(
                serde_json::to_value(b).unwrap().as_str().unwrap(),
                b.as_str()
            );
            let back: BarrierCode = serde_json::from_str(&format!("\"{}\"", b.as_str())).unwrap();
            assert_eq!(back, b);
        }
        assert!(
            serde_json::from_str::<BarrierCode>("\"too_busy\"").is_err(),
            "unknown codes are refused"
        );
    }

    #[test]
    fn test_cancelled_by_maps_to_the_legacy_reason() {
        assert_eq!(
            CancelledBy::Patient.legacy_reason(),
            CancellationReason::PatientRequest
        );
        assert_eq!(
            CancelledBy::Provider.legacy_reason(),
            CancellationReason::ProviderRequest
        );
        assert_eq!(
            CancelledBy::System.legacy_reason(),
            CancellationReason::Other
        );
    }
}
