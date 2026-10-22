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

use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::db::row::RowExt;
use crate::waits::WaitsService;
use crate::{Error, Result};

mod cohort;
mod decision;
mod read;
mod steps;

/// How long after a removal it can be undone.
pub const REINSTATE_DAYS: i64 = 90;
/// The most pathways one campaign may take.
pub const MAX_COHORT: i64 = 500;
pub const ANSWERS: [&str; 3] = ["still_needed", "no_longer_needed", "unsure"];
pub const CHANNELS: [&str; 5] = ["phone", "sms", "whatsapp", "email", "in_person"];

#[derive(Debug, Clone, Serialize)]
pub struct Campaign {
    pub id: Uuid,
    pub name: String,
    pub service_type: Option<String>,
    pub min_weeks: i32,
    pub started_by: String,
    pub reply_by: NaiveDate,
    pub final_wait_days: i32,
    pub status: String,
    pub created_at: DateTime<Utc>,
    /// Items by state (`selected`, `asked`, ...), for the campaign page.
    pub counts: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub pathway_id: Uuid,
    pub person_id: Uuid,
    pub target_service: String,
    pub state: String,
    pub answer: Option<String>,
    pub asked_at: Option<DateTime<Utc>>,
    pub letter_sent_at: Option<DateTime<Utc>>,
    pub final_by: Option<NaiveDate>,
    pub decided_by: Option<String>,
    pub removed_at: Option<DateTime<Utc>>,
    /// Why a person may act now, if they may: `letter_due`, `decision_due`.
    pub next: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewCampaign {
    pub name: String,
    pub started_by: String,
    pub service_type: Option<String>,
    /// Defaults to `PAS_CHECK_WEEKS`.
    pub min_weeks: Option<i64>,
    /// Days from now until the first reply is expected (default 14).
    pub reply_in_days: i64,
    /// Days to wait for a reply to the letter (default 14).
    pub final_wait_days: i64,
    /// How many pathways to take, longest-waiting first (1 to [`MAX_COHORT`]).
    pub limit: i64,
}

pub struct ListValidationService {
    db: DatabaseConnection,
    waits: std::sync::Arc<WaitsService>,
}

const ITEM_COLS: &str = "i.id, i.campaign_id, i.pathway_id, i.person_id, i.target_service, i.state, i.answer, \
    i.asked_at, i.letter_sent_at, i.final_by, i.decided_by, i.removed_at, \
    CASE WHEN i.state IN ('selected','asked') AND current_date > c.reply_by THEN 'letter_due' \
         WHEN i.state = 'answered' AND i.answer = 'no_longer_needed' THEN 'decision_due' \
         WHEN i.state = 'letter_sent' AND current_date > i.final_by THEN 'decision_due' END AS next";

impl ListValidationService {
    pub fn new(db: DatabaseConnection, waits: std::sync::Arc<WaitsService>) -> Self {
        Self { db, waits }
    }
}
