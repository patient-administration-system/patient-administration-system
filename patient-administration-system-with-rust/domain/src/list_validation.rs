//! Waiting-list validation (FR-170 to FR-174): the campaign and item states, the answers and channels, the shapes
//! the API sends, and the rules for when a keep or a removal may be decided (FR-172). Asking, recording and
//! removing need the database and live in the server's `list_validation` module, which re-exports this one.

use crate::{Error, Result};
use jiff::{Timestamp, civil::Date};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How long after a removal it can be undone.
pub const REINSTATE_DAYS: i64 = 90;

/// The most pathways one campaign may take.
pub const MAX_COHORT: i64 = 500;

pub const ANSWERS: [&str; 3] = ["still_needed", "no_longer_needed", "unsure"];

pub const CHANNELS: [&str; 5] = ["phone", "sms", "whatsapp", "email", "in_person"];

crate::string_enum! {
    /// Whether a campaign still takes steps (`validation_campaigns.status`).
    pub enum CampaignStatus {
        Open = "open",
        Closed = "closed",
    }
}

crate::string_enum! {
    /// Where one pathway is in its campaign (`validation_items.state`).
    pub enum ItemState {
        Selected = "selected",
        Asked = "asked",
        Answered = "answered",
        LetterSent = "letter_sent",
        Kept = "kept",
        Removed = "removed",
        Reinstated = "reinstated",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = list_validation::Campaign)]
pub struct Campaign {
    pub id: Uuid,
    pub name: String,
    #[schema(required = true)]
    pub service_type: Option<String>,
    pub min_weeks: i32,
    pub started_by: String,
    pub reply_by: Date,
    pub final_wait_days: i32,
    pub status: CampaignStatus,
    pub created_at: Timestamp,
    /// Items by state (`selected`, `asked`, ...), for the campaign page.
    #[schema(value_type = HashMap<String, i64>)]
    pub counts: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = list_validation::Item)]
pub struct Item {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub pathway_id: Uuid,
    pub person_id: Uuid,
    pub target_service: String,
    pub state: ItemState,
    #[schema(required = true)]
    pub answer: Option<String>,
    #[schema(required = true)]
    pub asked_at: Option<Timestamp>,
    #[schema(required = true)]
    pub letter_sent_at: Option<Timestamp>,
    #[schema(required = true)]
    pub final_by: Option<Date>,
    #[schema(required = true)]
    pub decided_by: Option<String>,
    #[schema(required = true)]
    pub removed_at: Option<Timestamp>,
    /// Why a person may act now, if they may: `letter_due`, `decision_due`.
    #[schema(required = true)]
    pub next: Option<String>,
}

/// Removals, reinstatements and keeps for one service in the period. No identifiers.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RemovalsByService {
    pub service: String,
    pub removed: i64,
    pub reinstated: i64,
    pub kept: i64,
}

/// The validation removals report: the period and a row per service.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RemovalsReport {
    pub from: Timestamp,
    pub to: Timestamp,
    pub by_service: Vec<RemovalsByService>,
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

/// What the decision rules need to know about an item.
pub struct DecisionRow {
    pub state: ItemState,
    pub answer: Option<String>,
    pub next: Option<String>,
    pub urgent: bool,
}

/// The rules (FR-172): may this decision be taken now? A removal is never for an urgent service nor for a
/// patient who said they still need the care or are unsure; either decision waits until the patient has said
/// the care is no longer needed, or the wait for a reply to the letter has ended.
pub fn check_decision_allowed(decision: &str, row: &DecisionRow) -> Result<()> {
    let state = row.state;
    if decision == "remove" {
        if row.urgent {
            return Err(Error::conflict(
                "an urgent pathway is never removed by validation: a clinician decides",
            ));
        }
        if matches!(row.answer.as_deref(), Some("still_needed" | "unsure")) {
            return Err(Error::conflict(
                "the patient said they still need this or are unsure: they are not removed",
            ));
        }
    }
    if row.next.as_deref() != Some("decision_due") {
        // A patient who answered still_needed may be recorded as kept at any time.
        let closing_a_yes = decision == "keep" && state == ItemState::Answered;
        if !closing_a_yes {
            return Err(Error::conflict(format!(
                "not ready for a decision: the item is {state}{}",
                if state == ItemState::LetterSent {
                    " and the wait for a reply to the letter has not ended"
                } else {
                    " and has no answer and no letter yet"
                }
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(state: &str, answer: Option<&str>, next: Option<&str>, urgent: bool) -> DecisionRow {
        DecisionRow {
            state: state.parse().expect("a state"),
            answer: answer.map(Into::into),
            next: next.map(Into::into),
            urgent,
        }
    }

    #[test]
    fn a_removal_needs_a_no_longer_needed_answer_or_an_unanswered_letter_that_ran_out() {
        // "No longer needed": decision due.
        assert!(
            check_decision_allowed(
                "remove",
                &row(
                    "answered",
                    Some("no_longer_needed"),
                    Some("decision_due"),
                    false
                )
            )
            .is_ok()
        );
        // Silent after the letter, wait over: decision due.
        assert!(
            check_decision_allowed(
                "remove",
                &row("letter_sent", None, Some("decision_due"), false)
            )
            .is_ok()
        );
        // Not yet: still waiting for the letter's reply, or no letter yet.
        assert!(check_decision_allowed("remove", &row("letter_sent", None, None, false)).is_err());
        assert!(check_decision_allowed("remove", &row("asked", None, None, false)).is_err());
        assert!(check_decision_allowed("keep", &row("asked", None, None, false)).is_err());
    }

    #[test]
    fn nobody_who_still_needs_the_care_or_is_unsure_is_removed_and_urgent_is_never_removed() {
        for answer in ["still_needed", "unsure"] {
            assert!(
                check_decision_allowed(
                    "remove",
                    &row("answered", Some(answer), Some("decision_due"), false)
                )
                .is_err()
            );
        }
        assert!(
            check_decision_allowed(
                "remove",
                &row(
                    "answered",
                    Some("no_longer_needed"),
                    Some("decision_due"),
                    true
                )
            )
            .is_err()
        );
        // Keeping someone who said they still need it is always allowed.
        assert!(
            check_decision_allowed("keep", &row("answered", Some("still_needed"), None, false))
                .is_ok()
        );
        assert!(
            check_decision_allowed("keep", &row("answered", Some("still_needed"), None, true))
                .is_ok()
        );
    }
}
