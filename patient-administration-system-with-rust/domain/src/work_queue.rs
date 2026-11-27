//! The staff work queue's rules (TSC-1): what kinds of work there are, which queue (team) each goes to, how
//! urgent it is and when it is due.
//!
//! The work itself is made by the features that already make it (a call to make, a follow-up to chase, a
//! waiting-list entry to validate, a patient's communication needs to review, a tolerance alert to look at).
//! The queue adds what they lack in common: an owner, a priority, a due date and one place to look. It is
//! support only (NFR-11): an item asks a person to act, and never acts on a patient or ranks patients.

use jiff::Timestamp;
use jiff::civil::Time;
use jiff::tz::TimeZone;

use crate::calendar::WorkingCalendar;
use crate::{Error, Result};

crate::string_enum! {
    /// What a work item is about, as `work_items.kind` stores it.
    pub enum WorkKind {
        /// A patient to phone (`staff_call_tasks`): silent at the second touch, asked to rebook, an urgent pathway.
        Call = "call",
        /// A missed or cancelled appointment to follow up (`follow_up_tasks`).
        FollowUp = "follow_up",
        /// A waiting-list entry to validate with the patient (`validation_items`).
        Validation = "validation",
        /// A patient's recorded communication needs to review with them (`patient_communication_profiles`).
        NeedsReview = "needs_review",
        /// A service measure outside its tolerance (`tolerance_alerts`).
        ToleranceAlert = "tolerance_alert",
        /// A patient we tried every way and heard nothing back from (TSC-8): the reminder ladder's call went
        /// unanswered, a message to them bounced, or a list-validation letter passed its reply-by date. A fact,
        /// not a prediction: nothing here guesses who will stop answering (NFR-11).
        Unreached = "unreached",
        /// A booked appointment a clinician's or room's absence touches (TSC-10): rebook it with the patient.
        /// Nothing was cancelled; the patient hears nothing until a person acts.
        AbsenceRebook = "absence_rebook",
    }
}

crate::string_enum! {
    /// What came of phoning a patient for an `unreached` item, as `unreached_call_outcomes.outcome` stores it.
    pub enum PhoneOutcome {
        Reached = "reached",
        NoAnswer = "no_answer",
        WrongNumber = "wrong_number",
    }
}

crate::string_enum! {
    /// Where a work item stands, as `work_items.status` stores it.
    pub enum WorkStatus {
        /// Nobody has it yet.
        Open = "open",
        /// A named person has it.
        Claimed = "claimed",
        /// Its source was closed (the call made, the follow-up recorded, the entry decided).
        Closed = "closed",
    }
}

/// Most urgent first: 1 is high, 3 is low.
pub const HIGH: i16 = 1;
pub const NORMAL: i16 = 2;
pub const LOW: i16 = 3;

/// A work item is due by the end of the working day it falls on.
fn end_of_day() -> Time {
    jiff::civil::time(17, 0, 0, 0)
}

impl WorkKind {
    /// The queue (team) that works this kind of item.
    pub fn queue(self) -> &'static str {
        match self {
            WorkKind::Call | WorkKind::FollowUp | WorkKind::Unreached | WorkKind::AbsenceRebook => {
                "booking"
            }
            WorkKind::Validation => "validation",
            WorkKind::NeedsReview => "communication",
            WorkKind::ToleranceAlert => "operations",
        }
    }

    /// Working days to act when the source names no due date of its own.
    pub fn working_days(self) -> u32 {
        match self {
            WorkKind::ToleranceAlert
            | WorkKind::Call
            | WorkKind::FollowUp
            | WorkKind::AbsenceRebook => 1,
            WorkKind::Unreached => 2,
            WorkKind::Validation | WorkKind::NeedsReview => 10,
        }
    }

    /// The default priority. A call about an urgent pathway is raised by the caller ([`call_priority`]).
    pub fn priority(self) -> i16 {
        match self {
            WorkKind::ToleranceAlert | WorkKind::AbsenceRebook => HIGH,
            WorkKind::Call | WorkKind::FollowUp | WorkKind::Unreached => NORMAL,
            WorkKind::Validation | WorkKind::NeedsReview => LOW,
        }
    }

    /// A short title, with nothing that identifies the patient.
    pub fn title(self) -> &'static str {
        match self {
            WorkKind::Call => "Ring the patient about their appointment",
            WorkKind::FollowUp => "Follow up a missed or cancelled appointment",
            WorkKind::Validation => "Check the patient still needs to wait",
            WorkKind::NeedsReview => "Review the patient's communication needs with them",
            WorkKind::ToleranceAlert => "A service measure is outside its tolerance",
            WorkKind::Unreached => "Try the patient by phone: nothing we sent has been answered",
            WorkKind::AbsenceRebook => {
                "Rebook with the patient: the clinician or room is unavailable"
            }
        }
    }

    /// Every queue, in a stable order.
    pub fn queues() -> Vec<&'static str> {
        let mut q: Vec<&'static str> = WorkKind::ALL.iter().map(|k| k.queue()).collect();
        q.sort_unstable();
        q.dedup();
        q
    }
}

/// A call's priority by its reason: an urgent pathway is high, the rest normal.
pub fn call_priority(reason: &str) -> i16 {
    if reason == "urgent_pathway" {
        HIGH
    } else {
        NORMAL
    }
}

/// The end (17:00 local) of the working day `days` working days after `from` in `tz`.
pub fn due_after(
    from: Timestamp,
    days: u32,
    calendar: &WorkingCalendar,
    tz: &TimeZone,
) -> Result<Timestamp> {
    let day = calendar.add_working_days(crate::time::date_in(from, tz), days)?;
    end_of(day, tz)
}

/// The end (17:00 local) of `day` in `tz`.
pub fn end_of(day: jiff::civil::Date, tz: &TimeZone) -> Result<Timestamp> {
    day.to_datetime(end_of_day())
        .to_zoned_datetime(tz.clone())
        .map(|z| z.timestamp())
        .map_err(|_| Error::validation("the due date is out of range"))
}

/// A queue name is one of [`WorkKind::queues`].
pub fn check_queue(queue: &str) -> Result<()> {
    if WorkKind::queues().contains(&queue) {
        Ok(())
    } else {
        Err(Error::validation(format!(
            "queue must be one of {}",
            WorkKind::queues().join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::utc;

    fn london() -> TimeZone {
        crate::time::zone(crate::config::env::DEFAULT_TIME_ZONE).unwrap()
    }

    #[test]
    fn every_kind_has_a_queue_a_title_and_a_priority_in_range() {
        for k in WorkKind::ALL {
            assert!(check_queue(k.queue()).is_ok(), "{k}");
            assert!(!k.title().is_empty());
            assert!((HIGH..=LOW).contains(&k.priority()), "{k}");
            assert!(k.working_days() >= 1);
        }
        assert_eq!(
            WorkKind::queues(),
            ["booking", "communication", "operations", "validation"]
        );
        assert!(check_queue("nobody").is_err());
    }

    #[test]
    fn an_urgent_pathway_call_is_high_and_the_others_normal() {
        assert_eq!(call_priority("urgent_pathway"), HIGH);
        assert_eq!(call_priority("silent"), NORMAL);
        assert_eq!(call_priority("reschedule_request"), NORMAL);
    }

    #[test]
    fn due_dates_count_working_days_and_end_at_five_local_time() {
        let cal = WorkingCalendar::default();
        // Friday 2026-11-20 10:00 UTC (London is UTC in winter) plus one working day: Monday 17:00.
        let due = due_after(utc(2026, 11, 20, 10, 0, 0), 1, &cal, &london()).unwrap();
        assert_eq!(due, utc(2026, 11, 23, 17, 0, 0));
        // In summer London is UTC+1, so 17:00 there is 16:00 UTC.
        let due = due_after(utc(2026, 7, 6, 9, 0, 0), 1, &cal, &london()).unwrap();
        assert_eq!(due, utc(2026, 7, 7, 16, 0, 0));
    }

    #[test]
    fn no_title_names_a_patient() {
        for k in WorkKind::ALL {
            let t = k.title();
            assert!(!t.contains('{') && !t.contains("name"), "{t}");
        }
    }
}
