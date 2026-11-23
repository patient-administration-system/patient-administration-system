//! The no-show probability (CAP-12, CAS-5): support only, interpretable, and gated by people.
//!
//! **What it is.** A transparent weighted sum, not a trained black box: a base rate, plus the patient's own
//! history of missed visits (shrunk towards the base rate when there are few past visits, so one miss out of
//! two is not read as 50% evidence), plus small adjustments for lead time and age band, and optional ones for
//! day of week and time of day (zero by default: they are the factors most open to steering unreliable slots
//! to particular groups). The weights are configuration (`PAS_NOSHOW_WEIGHTS`), so a clinician can read and
//! challenge them. The factors behind each number are stored and shown.
//!
//! **What it may do (NFR-11).** It may add support: put an appointment on the staff priority list so a person
//! rings the patient. It never removes, reorders away or withholds a slot, never overbooks, never delays a
//! contact, and acts on nothing by itself (UK GDPR Art. 22: staff decide). Nothing in the booking, rebooking,
//! standby or offer code reads it, a test proves they behave identically without it, and the schema guard
//! (`cap10_support_test`) lists its columns by name.
//!
//! **The gate.** It is off. It is on only when `PAS_NOSHOW_SCORE=true` *and* a named person has approved a
//! fairness review *and* a named person has approved a clinical safety review of this model version, and
//! nobody has since rejected either (a rejection switches it off at once). The fairness audit
//! ([`NoShowService::fairness_audit`]) backtests the model on past appointments by deprivation quintile and
//! language, with small cells withheld, so the reviewer has evidence. A risk score also re-opens the medical
//! device question and the clinical safety case (DTAC-1): a person must decide those, and this code cannot.
//!
//! **Not built:** distance from the clinic (needs a postcode-district band no one supplies yet), clinic type
//! as a factor, and weather or travel forecasts.

use crate::db::sql::Pool;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::analytics::{AnalyticsService, suppress};
use crate::config::env;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

mod assess;
mod call_list;
mod fairness;
mod review;
mod rollout;
pub use call_list::CallItem;
pub use fairness::{FairnessAudit, FairnessCell, FairnessOverall, ModelBands, ModelCard};
// The model, its inputs, bands and the shapes it is shown in are domain rules (QUAL-52).
pub use patient_administration_system_domain::noshow::*;
pub use review::{ReviewKind, ReviewOutcome};
pub use rollout::{
    CalibrationCell, ClinicEffect, Evaluation, RateWindow, RolloutRow, RolloutStage,
};

/// A fairness review stays valid this long; clinical safety, a year.
const FAIRNESS_VALID_DAYS: i64 = 180;
const SAFETY_VALID_DAYS: i64 = 365;

pub struct NoShowService {
    db: Pool,
}

impl NoShowService {
    pub fn new(db: Pool) -> Self {
        Self { db }
    }

    /// The cheap check before any work: is the switch on? (It is not enough on its own; see [`Self::status`].)
    pub fn switched_on() -> Result<bool> {
        Ok(env::flag("PAS_NOSHOW_SCORE")?)
    }
}
