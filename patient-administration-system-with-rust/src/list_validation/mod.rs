//! Waiting-list validation campaigns (NEW-2).
//!
//! A campaign takes the pathways that have waited longest and asks, one by one, whether the care is still
//! needed (the CAP-8 "still needed?" check, made in bulk). The rules, each tested:
//!
//! - **Nothing removes anyone by itself.** A removal is a named person's decision, taken after the patient
//!   said the care is no longer needed, or after they were asked, did not answer, were sent a letter and the
//!   wait for a reply to it ran out. The code will not let that step be skipped.
//! - **A patient who said they still need it, or are unsure, is never removed** by the campaign.
//! - **Urgent pathways are never in a campaign.** A service listed for human follow-up
//!   ([`crate::follow_up`]) is left out of the cohort and refused at the decision.
//! - **A removal is recorded as one** (state `removed`, the clock stopped with the reason
//!   `validation_removal`), so it is reported apart from treatment, and it can be **reinstated** within
//!   [`REINSTATE_DAYS`]: the waiting-list entry goes back and the clock continues as if nothing had happened.
//!
//! Nothing here contacts the patient: until a carrier exists (NOT-3) a person asks, by phone or text, and
//! records it. The letter is posted by a person and recorded.

use crate::db::sql::Pool;
use jiff::Timestamp;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::waits::WaitsService;
use crate::{Error, Result};

// The states, answers, shapes and decision rules are domain rules (QUAL-52).
pub use patient_administration_system_domain::list_validation::*;

mod cohort;
mod decision;
mod read;
mod steps;

pub struct ListValidationService {
    db: Pool,
    waits: std::sync::Arc<WaitsService>,
}

const ITEM_COLS: &str = "i.id, i.campaign_id, i.pathway_id, i.person_id, i.target_service, i.state, i.answer, \
    i.asked_at, i.letter_sent_at, i.final_by, i.decided_by, i.removed_at, \
    CASE WHEN i.state IN ('selected','asked') AND current_date > c.reply_by THEN 'letter_due' \
         WHEN i.state = 'answered' AND i.answer = 'no_longer_needed' THEN 'decision_due' \
         WHEN i.state = 'letter_sent' AND current_date > i.final_by THEN 'decision_due' END AS next";

impl ListValidationService {
    pub fn new(db: Pool, waits: std::sync::Arc<WaitsService>) -> Self {
        Self { db, waits }
    }
}
