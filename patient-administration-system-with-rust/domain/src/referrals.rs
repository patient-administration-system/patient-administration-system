//! Referrals: where they come from, where they have got to, the legal moves between those states (PCC-2), and
//! the shapes the API sends. Receiving, triaging, waitlisting and booking need the database and live in the
//! server's `referrals` module, which re-exports this one.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::appointment::Appointment;

crate::string_enum! {
    /// Where a referral came from.
    pub enum ReferralSource {
        Gp = "gp",
        ERs = "e-rs",
        Internal = "internal",
        /// An electronic patient record, sending its own referral id as `external_ref` (WPAS-9).
        Epr = "epr",
    }
}

crate::string_enum! {
    /// Where a referral has got to.
    pub enum ReferralStatus {
        Received = "received",
        Waitlisted = "waitlisted",
        Booked = "booked",
        Closed = "closed",
        ReturnedWithAdvice = "returned_with_advice",
        Redirected = "redirected",
    }
}

crate::string_enum! {
    /// Triage outcomes (NEW-11). Both end the referral at this service, and each is stored as the referral's
    /// status (QUAL-28): [`TriageOutcome::status`].
    pub enum TriageOutcome {
        ReturnedWithAdvice = "returned_with_advice",
        Redirected = "redirected",
    }
}

impl TriageOutcome {
    /// The status a referral takes when it is triaged so.
    pub fn status(self) -> ReferralStatus {
        match self {
            Self::ReturnedWithAdvice => ReferralStatus::ReturnedWithAdvice,
            Self::Redirected => ReferralStatus::Redirected,
        }
    }
}

/// A referral in one of these states cannot be booked, waitlisted, closed or triaged again.
pub fn is_ended(status: ReferralStatus) -> bool {
    use ReferralStatus::*;
    matches!(status, Booked | Closed | ReturnedWithAdvice | Redirected)
}

/// The legal moves of a referral (PCC-2), as one table. The operations keep their own checks; the database test
/// `every_move_of_the_lifecycle_table_is_allowed_or_refused_as_written` holds them to this.
///
/// - `received` goes to `waitlisted`, `booked`, `closed`, `returned_with_advice` or `redirected`.
/// - `waitlisted` goes to `booked` or `closed` (not back to `received`, not waitlisted twice, not triaged).
/// - `booked` goes to `closed` only.
/// - `closed`, `returned_with_advice` and `redirected` are final.
pub fn is_legal_move(from: ReferralStatus, to: ReferralStatus) -> bool {
    use ReferralStatus::*;
    matches!(
        (from, to),
        (
            Received,
            Waitlisted | Booked | Closed | ReturnedWithAdvice | Redirected
        ) | (Waitlisted, Booked | Closed)
            | (Booked, Closed)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Referral {
    pub id: Uuid,
    pub person_id: Uuid,
    pub source: ReferralSource,
    #[schema(required = true)]
    pub external_ref: Option<String>,
    pub target_service: String,
    #[schema(required = true)]
    pub reason: Option<String>,
    #[schema(required = true)]
    pub referring_worker_id: Option<Uuid>,
    pub status: ReferralStatus,
    pub received_at: Timestamp,
    /// The appointment booked from this referral, if any.
    #[schema(required = true)]
    pub appointment_id: Option<Uuid>,
    /// Set when a consultant triaged the referral (`returned_with_advice` or `redirected`).
    #[schema(required = true)]
    pub triaged_by: Option<String>,
    #[schema(required = true)]
    pub triaged_at: Option<Timestamp>,
    /// For a redirect: the service it went to and the new referral that carries the patient there.
    #[schema(required = true)]
    pub redirected_to_service: Option<String>,
    #[schema(required = true)]
    pub redirected_referral_id: Option<Uuid>,
    /// The record's version (OFH-11): sent back as `If-Match` to close or triage it. Also returned as `ETag`.
    pub updated_at: Timestamp,
}

pub struct NewReferral {
    pub person_id: Uuid,
    pub source: String,
    pub external_ref: Option<String>,
    pub target_service: String,
    pub reason: Option<String>,
    pub referring_worker_id: Option<Uuid>,
    pub received_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Booked {
    pub referral: Referral,
    pub appointment: Appointment,
}

/// What creating a referral answers: the referral, and whether it was one already received (same source and
/// external reference), in which case nothing new was made.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Created {
    pub referral: Referral,
    pub duplicate: bool,
}

#[cfg(test)]
mod lifecycle_tests {
    use super::{ReferralStatus, ReferralStatus::*, TriageOutcome, is_ended, is_legal_move};

    /// QUAL-28: a triage outcome is stored as the referral's status, so each must be a status, the same word,
    /// and one that ends the referral.
    #[test]
    fn every_triage_outcome_is_an_ending_status_of_the_same_name() {
        for outcome in TriageOutcome::ALL {
            let status = outcome.status();
            assert_eq!(status.as_str(), outcome.as_str());
            assert!(is_ended(status), "{status}");
        }
    }

    #[test]
    fn the_table_has_eight_legal_moves_and_nothing_leaves_a_final_state() {
        let legal = ReferralStatus::ALL
            .iter()
            .flat_map(|f| ReferralStatus::ALL.iter().map(move |t| (*f, *t)))
            .filter(|(f, t)| is_legal_move(*f, *t))
            .count();
        assert_eq!(legal, 5 + 2 + 1);
        for f in [Closed, ReturnedWithAdvice, Redirected] {
            assert!(ReferralStatus::ALL.iter().all(|t| !is_legal_move(f, *t)));
        }
        assert!(
            ReferralStatus::ALL
                .iter()
                .all(|s| !is_legal_move(*s, Received))
        );
        assert!(ReferralStatus::ALL.iter().all(|s| !is_legal_move(*s, *s)));
    }

    #[test]
    fn a_booked_referral_is_ended_for_booking_but_can_still_be_closed() {
        assert!(is_ended(Booked) && is_legal_move(Booked, Closed));
    }
}
