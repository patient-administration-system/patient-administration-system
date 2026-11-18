//! The standby list and slot releases: who is waiting for a short-notice slot, what was released, and the
//! checks on joining. Keeping the list and releasing slots need the database and live in the server's `standby`
//! module, which re-exports this one.

use crate::{Error, Result};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StandbyEntry {
    pub id: Uuid,
    pub person_id: Uuid,
    pub service_type: String,
    pub short_notice_ok: bool,
    /// A locale code such as `cy-001`: the language the patient wants to be offered care in.
    pub language: Option<String>,
    pub needs_transport: bool,
    pub phone_only: bool,
    pub active: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct StandbyInput {
    pub person_id: Uuid,
    pub service_type: String,
    #[serde(default = "yes")]
    pub short_notice_ok: bool,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub needs_transport: bool,
    #[serde(default)]
    pub phone_only: bool,
}

pub fn yes() -> bool {
    true
}

impl StandbyInput {
    pub fn validate(&self) -> Result<()> {
        let st = self.service_type.trim();
        if st.is_empty() || st.len() > 80 {
            return Err(Error::validation("service_type must be 1 to 80 characters"));
        }
        if let Some(l) = &self.language
            && !(2..=12).contains(&l.len())
        {
            return Err(Error::validation(
                "language must be a locale code such as cy-001",
            ));
        }
        Ok(())
    }
}

crate::string_enum! {
    /// Where a released slot stands.
    pub enum ReleaseStatus {
        Open = "open",
        Refilled = "refilled",
        /// Derived, never stored: still open after the slot's start time has passed.
        Expired = "expired",
    }
}

impl ReleaseStatus {
    /// The values `slot_releases.status` stores; `expired` is only ever derived.
    pub const STORED: &'static [&'static str] = &["open", "refilled"];
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SlotRelease {
    pub id: Uuid,
    pub slot_id: Uuid,
    pub appointment_id: Uuid,
    pub service_type: Option<String>,
    pub start_datetime: Timestamp,
    pub released_at: Timestamp,
    pub notice_days: Option<i32>,
    pub release_class: crate::models::appointment::ReleaseClass,
    pub status: ReleaseStatus,
    pub refilled_by_appointment_id: Option<Uuid>,
    pub refilled_at: Option<Timestamp>,
    pub seconds_to_refill: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReleaseSummary {
    pub total: i64,
    pub open: i64,
    pub refilled: i64,
    pub expired: i64,
    /// Median seconds from release to refill, over refilled releases. `None` when there are none.
    pub median_seconds_to_refill: Option<f64>,
}

/// What `GET /api/slot-releases` sends: the counts and the releases they count.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Releases {
    pub summary: ReleaseSummary,
    pub releases: Vec<SlotRelease>,
}
