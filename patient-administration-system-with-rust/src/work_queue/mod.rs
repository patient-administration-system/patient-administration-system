//! One staff work queue (TSC-1).
//!
//! The PAS already makes the right work for people, but in separate lists: calls to make, follow-ups to chase,
//! waiting-list entries to validate, communication needs to review, tolerance alerts. This module keeps one
//! work item for every open item of those lists, with what they lack in common: a queue (team), an owner or
//! none, a priority and a due date. [`WorkQueueService::sync`] opens an item when its source opens and closes
//! it when its source closes; a person claims, releases, reassigns or takes the next item.
//!
//! **Support only (NFR-11).** A work item is closed only by its source being closed, in that feature's own
//! place and by its own rules (the call's outcome recorded, the follow-up's outcome, the validation decided);
//! nothing here changes an appointment, a referral or a pathway, and nothing ranks patients. The order of a
//! queue is priority, then due date, then age: facts about the work, not about the patient.
//! `tests/tsc1_work_queue_test.rs` checks the source of this module for any write to those tables.

use std::collections::BTreeSet;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;
use uuid::Uuid;

use crate::calendar::WorkingCalendar;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::row::RowExt;
use crate::db::sql::{Conn, FromRow, Pool, Row, Stmt, Transact};
use crate::{Error, Result};

pub use patient_administration_system_domain::work_queue::*;

/// One work item, as the API sends it.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct WorkItem {
    pub id: Uuid,
    #[schema(value_type = String)]
    pub kind: WorkKind,
    /// The row of the source list this item stands for.
    pub source_id: Uuid,
    #[schema(required = true)]
    pub person_id: Option<Uuid>,
    /// `appointment`, `pathway`, `profile` or `alert`.
    #[schema(required = true)]
    pub subject_kind: Option<String>,
    #[schema(required = true)]
    pub subject_id: Option<Uuid>,
    pub queue: String,
    /// 1 is high, 3 is low.
    pub priority: i16,
    pub due_at: Timestamp,
    /// Due before now and not closed.
    pub overdue: bool,
    #[schema(value_type = String)]
    pub status: WorkStatus,
    #[schema(required = true)]
    pub owner: Option<String>,
    pub title: String,
    pub opened_at: Timestamp,
    #[schema(required = true)]
    pub claimed_at: Option<Timestamp>,
    #[schema(required = true)]
    pub closed_at: Option<Timestamp>,
}

impl FromRow for WorkItem {
    fn from_row(r: &Row) -> Result<Self> {
        let kind: WorkKind = r.col::<String>("kind")?.parse()?;
        let status: WorkStatus = r.col::<String>("status")?.parse()?;
        let due_at: Timestamp = r.col("due_at")?;
        Ok(WorkItem {
            id: r.col("id")?,
            kind,
            source_id: r.col("source_id")?,
            person_id: r.col("person_id")?,
            subject_kind: r.col("subject_kind")?,
            subject_id: r.col("subject_id")?,
            queue: r.col("queue")?,
            priority: r.col("priority")?,
            overdue: status != WorkStatus::Closed && due_at < Timestamp::now(),
            due_at,
            status,
            owner: r.col("owner")?,
            title: kind.title().to_string(),
            opened_at: r.col("opened_at")?,
            claimed_at: r.col("claimed_at")?,
            closed_at: r.col("closed_at")?,
        })
    }
}

/// What a list asks for.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub queue: Option<String>,
    /// Only items this person has.
    pub owner: Option<String>,
    /// Only items nobody has.
    pub unclaimed: bool,
    /// Only items due before now.
    pub overdue: bool,
}

/// A person's reminders: their items already past due, and those due by the end of the current working day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Reminders {
    pub overdue: Vec<WorkItem>,
    pub due_today: Vec<WorkItem>,
    /// The end of the working day the reminders are counted to.
    pub due_by: Timestamp,
}

/// What one sync did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SyncReport {
    pub opened: usize,
    pub reopened: usize,
    pub closed: usize,
    pub open_now: usize,
}

/// One open item of a source list.
struct Open {
    source_id: Uuid,
    person_id: Option<Uuid>,
    subject_kind: Option<&'static str>,
    subject_id: Option<Uuid>,
    /// The source's own due date, when it has one.
    due: Option<Timestamp>,
    /// When the work started, for a due date counted in working days.
    since: Timestamp,
    priority: i16,
}

pub struct WorkQueueService {
    db: Pool,
    /// The working calendar, or why the setting for it cannot be used (start-up refuses it; a service built
    /// anyway reports it on first use instead of using a made-up calendar, RCA-IN-2).
    calendar: std::result::Result<WorkingCalendar, String>,
    tz: TimeZone,
}

const SELECT: &str = "SELECT id, kind, source_id, person_id, subject_kind, subject_id, queue, priority, due_at, \
     status, owner, opened_at, claimed_at, closed_at FROM work_items";

impl WorkQueueService {
    fn calendar(&self) -> Result<&WorkingCalendar> {
        self.calendar.as_ref().map_err(|e| Error::config(e.clone()))
    }

    pub fn new(db: Pool) -> Self {
        Self {
            db,
            calendar: WorkingCalendar::from_env().map_err(|e| e.to_string()),
            tz: crate::inbound::messages::timezone(),
        }
    }

    /// Bring the queue in line with its sources: an item for every open source row, closed when the source
    /// closed, reopened when it opened again.
    pub async fn sync(&self, now: Timestamp) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        for kind in WorkKind::ALL {
            let open = self.open_of(*kind, now).await?;
            let r = self.apply(*kind, &open, now).await?;
            report.opened += r.opened;
            report.reopened += r.reopened;
            report.closed += r.closed;
            report.open_now += open.len();
        }
        Ok(report)
    }

    async fn open_of(&self, kind: WorkKind, now: Timestamp) -> Result<Vec<Open>> {
        let rows = match kind {
            WorkKind::Call => {
                self.db
                    .all(Stmt::text(
                        "SELECT id, person_id, appointment_id, reason, due_at, created_at FROM staff_call_tasks \
                         WHERE handled_at IS NULL",
                    ))
                    .await?
            }
            WorkKind::FollowUp => {
                self.db
                    .all(Stmt::text(
                        "SELECT id, person_id, appointment_id, due_at, created_at FROM follow_up_tasks \
                         WHERE outcome IS NULL",
                    ))
                    .await?
            }
            WorkKind::Validation => {
                self.db
                    .all(Stmt::text(
                        "SELECT id, person_id, pathway_id, final_by FROM validation_items \
                         WHERE state IN ('selected','asked','answered','letter_sent')",
                    ))
                    .await?
            }
            WorkKind::NeedsReview => {
                let cutoff =
                    crate::time::sub_months_utc(now, crate::needs_review::DEFAULT_MONTHS)
                        .unwrap_or(now);
                self.db
                    .all(Stmt::new(
                        "SELECT person_id FROM patient_communication_profiles \
                         WHERE cardinality(accessibility_needs) > 0 \
                           AND (needs_reviewed_at IS NULL OR needs_reviewed_at < $1)",
                        [cutoff.into()],
                    ))
                    .await?
            }
            WorkKind::ToleranceAlert => {
                self.db
                    .all(Stmt::text(
                        "SELECT id, raised_at FROM tolerance_alerts \
                         WHERE cleared_at IS NULL AND acknowledged_at IS NULL",
                    ))
                    .await?
            }
        };
        rows.iter().map(|r| self.open_row(kind, r, now)).collect()
    }

    fn open_row(&self, kind: WorkKind, r: &Row, now: Timestamp) -> Result<Open> {
        Ok(match kind {
            WorkKind::Call => {
                let reason: String = r.col("reason")?;
                Open {
                    source_id: r.col("id")?,
                    person_id: r.col("person_id")?,
                    subject_kind: Some("appointment"),
                    subject_id: r.col("appointment_id")?,
                    due: Some(r.col("due_at")?),
                    since: r.col("created_at")?,
                    priority: call_priority(&reason),
                }
            }
            WorkKind::FollowUp => Open {
                source_id: r.col("id")?,
                person_id: r.col("person_id")?,
                subject_kind: Some("appointment"),
                subject_id: r.col("appointment_id")?,
                due: Some(r.col("due_at")?),
                since: r.col("created_at")?,
                priority: kind.priority(),
            },
            WorkKind::Validation => {
                let final_by: Option<jiff::civil::Date> = r.col("final_by")?;
                Open {
                    source_id: r.col("id")?,
                    person_id: r.col("person_id")?,
                    subject_kind: Some("pathway"),
                    subject_id: r.col("pathway_id")?,
                    due: final_by.map(|d| end_of(d, &self.tz)).transpose()?,
                    since: now,
                    priority: kind.priority(),
                }
            }
            WorkKind::NeedsReview => {
                let person: Uuid = r.col("person_id")?;
                Open {
                    source_id: person,
                    person_id: Some(person),
                    subject_kind: Some("profile"),
                    subject_id: Some(person),
                    due: None,
                    since: now,
                    priority: kind.priority(),
                }
            }
            WorkKind::ToleranceAlert => {
                let id: Uuid = r.col("id")?;
                Open {
                    source_id: id,
                    person_id: None,
                    subject_kind: Some("alert"),
                    subject_id: Some(id),
                    due: None,
                    since: r.col("raised_at")?,
                    priority: kind.priority(),
                }
            }
        })
    }

    async fn apply(&self, kind: WorkKind, open: &[Open], now: Timestamp) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        // Read once and compare here, so a sync where nothing changed sends no write at all (PERF-3 found the
        // first version sending one statement per open item, every minute).
        // The items still open, and the closed ones whose source is open again; closed history is not read.
        let open_ids: BTreeSet<Uuid> = open.iter().map(|o| o.source_id).collect();
        let open_list: Vec<Uuid> = open_ids.iter().copied().collect();
        let existing: Vec<(Uuid, (String, Timestamp, i16))> = self
            .db
            .all(Stmt::new(
                "SELECT source_id, status, due_at, priority FROM work_items WHERE kind = $1 \
                 AND (status <> 'closed' OR source_id = ANY($2))",
                [kind.as_str().into(), open_list.into()],
            ))
            .await?
            .iter()
            .map(|r| {
                Ok((
                    r.col("source_id")?,
                    (r.col("status")?, r.col("due_at")?, r.col("priority")?),
                ))
            })
            .collect::<Result<_>>()?;
        let known: std::collections::BTreeMap<Uuid, (String, Timestamp, i16)> =
            existing.into_iter().collect();
        let txn = self.db.begin_txn().await?;
        for o in open {
            let due = match o.due {
                Some(d) => d,
                None => due_after(o.since, kind.working_days(), self.calendar()?, &self.tz)?,
            };
            match known
                .get(&o.source_id)
                .map(|(s, d, p)| (s.as_str(), *d, *p))
            {
                None => {
                    txn.run(Stmt::new(
                        "INSERT INTO work_items (id, kind, source_id, person_id, subject_kind, subject_id, queue, \
                         priority, due_at, opened_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) \
                         ON CONFLICT (kind, source_id) DO NOTHING",
                        [
                            Uuid::new_v4().into(),
                            kind.as_str().into(),
                            o.source_id.into(),
                            o.person_id.into(),
                            o.subject_kind.map(str::to_string).into(),
                            o.subject_id.into(),
                            kind.queue().into(),
                            o.priority.into(),
                            due.into(),
                            now.into(),
                        ],
                    ))
                    .await?;
                    report.opened += 1;
                }
                Some(("closed", _, _)) => {
                    txn.run(Stmt::new(
                        "UPDATE work_items SET status = 'open', owner = NULL, claimed_at = NULL, closed_at = NULL, \
                         opened_at = $3, due_at = $4, priority = $5 WHERE kind = $1 AND source_id = $2",
                        [
                            kind.as_str().into(),
                            o.source_id.into(),
                            now.into(),
                            due.into(),
                            o.priority.into(),
                        ],
                    ))
                    .await?;
                    report.reopened += 1;
                }
                Some((_, was_due, was_priority))
                    if o.due.is_some() && (was_due != due || was_priority != o.priority) =>
                {
                    // A source due date can move (a call rescheduled); keep the item in step.
                    txn.run(Stmt::new(
                        "UPDATE work_items SET due_at = $3, priority = $4 WHERE kind = $1 AND source_id = $2",
                        [
                            kind.as_str().into(),
                            o.source_id.into(),
                            due.into(),
                            o.priority.into(),
                        ],
                    ))
                    .await?;
                }
                Some(_) => {}
            }
        }
        let gone: Vec<Uuid> = known
            .iter()
            .filter(|(id, (status, _, _))| status.as_str() != "closed" && !open_ids.contains(*id))
            .map(|(id, _)| *id)
            .collect();
        if !gone.is_empty() {
            report.closed = txn
                .run(Stmt::new(
                    "UPDATE work_items SET status = 'closed', closed_at = $3 \
                     WHERE kind = $1 AND source_id = ANY($2) AND status <> 'closed'",
                    [kind.as_str().into(), gone.into(), now.into()],
                ))
                .await?
                .rows_affected() as usize;
        }
        txn.commit().await.map_err(Error::Database)?;
        Ok(report)
    }

    /// Open and claimed items that match `filter`: most urgent first, then soonest due, then oldest.
    pub async fn list(
        &self,
        filter: &Filter,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<WorkItem>> {
        if let Some(q) = &filter.queue {
            check_queue(q)?;
        }
        let rows = rows.into();
        self.db
            .all_as::<WorkItem>(Stmt::new(
                format!(
                    "{SELECT} WHERE status <> 'closed' AND ($1::text IS NULL OR queue = $1) \
                     AND ($2::text IS NULL OR owner = $2) AND (NOT $3 OR owner IS NULL) \
                     AND (NOT $4 OR due_at < now()) \
                     ORDER BY priority, due_at, opened_at, id LIMIT $5 OFFSET $6"
                ),
                [
                    filter.queue.clone().into(),
                    filter.owner.clone().into(),
                    filter.unclaimed.into(),
                    filter.overdue.into(),
                    rows.fetch()?.into(),
                    rows.offset()?.into(),
                ],
            ))
            .await
    }

    /// `owner`'s items due by the end of the current working day (17:00 local; on a day off, the next working
    /// day's), split into those already past due and the rest: the reminder shown before an item is due.
    pub async fn reminders(&self, owner: &str, now: Timestamp) -> Result<Reminders> {
        let owner = crate::validation::named_person(owner, "reminders are for a signed-in person")?;
        let today = crate::time::date_in(now, &self.tz);
        let mut due_by = end_of(self.calendar()?.add_working_days(today, 0)?, &self.tz)?;
        if due_by <= now {
            // After 17:00: the next working day's end.
            due_by = due_after(now, 1, self.calendar()?, &self.tz)?;
        }
        let items = self
            .db
            .all_as::<WorkItem>(Stmt::new(
                format!(
                    "{SELECT} WHERE status = 'claimed' AND owner = $1 AND due_at <= $2 \
                     ORDER BY due_at, priority, id LIMIT 200"
                ),
                [owner.into(), due_by.into()],
            ))
            .await?;
        let (overdue, due_today) = items.into_iter().partition(|w| w.due_at < now);
        Ok(Reminders {
            overdue,
            due_today,
            due_by,
        })
    }

    pub async fn get(&self, id: Uuid) -> Result<WorkItem> {
        self.db
            .one_as::<WorkItem>(Stmt::new(format!("{SELECT} WHERE id = $1"), [id.into()]))
            .await?
            .ok_or_else(|| Error::not_found(format!("work item {id}")))
    }

    /// `who` takes an open item. Taking one you already have is no change; one someone else has, or one
    /// closed, is a conflict.
    pub async fn claim(&self, id: Uuid, who: &str, ctx: &UserContext) -> Result<WorkItem> {
        let who = crate::validation::named_person(who, "claiming needs a signed-in person")?;
        let before = self.get(id).await?;
        match (before.status, before.owner.as_deref()) {
            (WorkStatus::Claimed, Some(o)) if o == who => return Ok(before),
            (WorkStatus::Claimed, _) => {
                return Err(Error::conflict(
                    "someone else has this item; ask them, or reassign it",
                ));
            }
            (WorkStatus::Closed, _) => return Err(Error::conflict("this item is closed")),
            (WorkStatus::Open, _) => {}
        }
        self.set_owner(id, Some(who), "claim", &before, ctx, "status = 'open'")
            .await
    }

    /// The owner gives an item back to its queue.
    pub async fn release(&self, id: Uuid, who: &str, ctx: &UserContext) -> Result<WorkItem> {
        let who = crate::validation::named_person(who, "releasing needs a signed-in person")?;
        let before = self.get(id).await?;
        if before.status != WorkStatus::Claimed || before.owner.as_deref() != Some(who) {
            return Err(Error::conflict(
                "only the person who has an item can release it",
            ));
        }
        self.set_owner(id, None, "release", &before, ctx, "status = 'claimed'")
            .await
    }

    /// Give an open or claimed item to `to` (a named person).
    pub async fn assign(&self, id: Uuid, to: &str, ctx: &UserContext) -> Result<WorkItem> {
        let to = crate::validation::named_person(to, "assign to a named person")?;
        let before = self.get(id).await?;
        if before.status == WorkStatus::Closed {
            return Err(Error::conflict("this item is closed"));
        }
        self.set_owner(id, Some(to), "assign", &before, ctx, "status <> 'closed'")
            .await
    }

    /// `who` takes the next open item of `queue`: the most urgent, then the soonest due, then the oldest.
    /// `None` when the queue is empty. Two people asking at once get different items.
    pub async fn take_next(
        &self,
        queue: &str,
        who: &str,
        ctx: &UserContext,
    ) -> Result<Option<WorkItem>> {
        check_queue(queue)?;
        let who = crate::validation::named_person(who, "taking work needs a signed-in person")?;
        let txn = self.db.begin_txn().await?;
        let Some(next) = txn
            .one_as::<WorkItem>(Stmt::new(
                format!(
                    "{SELECT} WHERE queue = $1 AND status = 'open' \
                     ORDER BY priority, due_at, opened_at, id LIMIT 1 FOR UPDATE SKIP LOCKED"
                ),
                [queue.into()],
            ))
            .await?
        else {
            return Ok(None);
        };
        let after = Self::write_owner(&txn, next.id, Some(who), "status = 'open'").await?;
        AuditLogRepository::log(
            &txn,
            "work_item",
            next.id,
            "take_next",
            Some(serde_json::json!({ "status": next.status.as_str(), "owner": next.owner })),
            Some(serde_json::json!({ "status": after.status.as_str(), "owner": after.owner })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(Some(after))
    }

    async fn set_owner(
        &self,
        id: Uuid,
        owner: Option<&str>,
        action: &str,
        before: &WorkItem,
        ctx: &UserContext,
        guard: &str,
    ) -> Result<WorkItem> {
        let txn = self.db.begin_txn().await?;
        let after = Self::write_owner(&txn, id, owner, guard).await?;
        AuditLogRepository::log(
            &txn,
            "work_item",
            id,
            action,
            Some(serde_json::json!({ "status": before.status.as_str(), "owner": before.owner })),
            Some(serde_json::json!({ "status": after.status.as_str(), "owner": after.owner })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(after)
    }

    /// Set the owner (claimed) or clear it (open), if the row still matches `guard`.
    async fn write_owner<C: Conn>(
        conn: &C,
        id: Uuid,
        owner: Option<&str>,
        guard: &str,
    ) -> Result<WorkItem> {
        let sql = match owner {
            Some(_) => format!(
                "UPDATE work_items SET status = 'claimed', owner = $2, claimed_at = now() \
                 WHERE id = $1 AND {guard} RETURNING *"
            ),
            None => format!(
                "UPDATE work_items SET status = 'open', owner = NULL, claimed_at = NULL \
                 WHERE id = $1 AND {guard} RETURNING *"
            ),
        };
        let params = match owner {
            Some(o) => vec![id.into(), o.into()],
            None => vec![id.into()],
        };
        conn.one_as::<WorkItem>(Stmt::new(sql, params))
            .await?
            .ok_or_else(|| Error::conflict("the item changed meanwhile; read it again"))
    }
}
