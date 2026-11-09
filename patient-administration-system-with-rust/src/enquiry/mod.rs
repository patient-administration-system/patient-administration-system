//! Patient enquiry (WPAS-3, FR-217): what the PAS holds for one person, from one route.
//!
//! Staff answering a phone call need one place to see a patient's appointments, waits, referrals, suspected
//! conditions, admissions and letters. This assembles the reads that already exist; it adds no data. A
//! letter is summarised (channel, status, dates), never its body, which the letter routes serve.
//!
//! The route is under `/api/patients/{id}/`, so the read audit writes one `read` row for the enquiry
//! (NEW-9): one access, one row.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;
use crate::db::repositories::admission::AdmissionRepository;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::letter::LetterRepository;
use crate::db::repositories::rtt::RttRepository;
use crate::db::repositories::waitlist::WaitlistRepository;
use crate::db::sql::Pool;
use crate::models::admission::Admission;
use crate::models::appointment::Appointment;
use crate::models::communication::{DeliveryChannel, LetterStatus};
use crate::models::rtt::RTTPathway;
use crate::models::waitlist::WaitlistEntry;
use crate::referrals::{Referral, ReferralService};
use crate::suspicions::{Suspicion, SuspicionService};

/// A letter without its words.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LetterSummary {
    pub id: Uuid,
    #[schema(required = true)]
    pub appointment_id: Option<Uuid>,
    pub channel: DeliveryChannel,
    pub status: LetterStatus,
    pub created_at: DateTime<Utc>,
    #[schema(required = true)]
    pub sent_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Enquiry {
    pub person_id: Uuid,
    pub appointments: Vec<Appointment>,
    pub waitlist_entries: Vec<WaitlistEntry>,
    pub rtt_pathways: Vec<RTTPathway>,
    pub referrals: Vec<Referral>,
    pub suspected_conditions: Vec<Suspicion>,
    pub admissions: Vec<Admission>,
    pub letters: Vec<LetterSummary>,
}

/// Everything the PAS holds for a person. An unknown person is not an error: the PAS does not own the
/// patient record (person-service does), so it answers with empty lists.
pub async fn enquiry(
    db: &Pool,
    referrals: &ReferralService,
    suspicions: &SuspicionService,
    person_id: Uuid,
) -> Result<Enquiry> {
    let letters = LetterRepository::list_for_person(db, person_id, None)
        .await?
        .into_iter()
        .map(|l| LetterSummary {
            id: l.id,
            appointment_id: l.appointment_id,
            channel: l.channel,
            status: l.status,
            created_at: l.created_at,
            sent_at: l.sent_at,
        })
        .collect();
    Ok(Enquiry {
        person_id,
        appointments: AppointmentRepository::list_by_patient(db, person_id).await?,
        waitlist_entries: WaitlistRepository::list_by_person(db, person_id).await?,
        rtt_pathways: RttRepository::list_pathways_by_patient(db, person_id).await?,
        referrals: referrals.for_person(person_id).await?,
        suspected_conditions: suspicions.for_person(person_id).await?,
        admissions: AdmissionRepository::list_by_patient(db, person_id).await?,
        letters,
    })
}
