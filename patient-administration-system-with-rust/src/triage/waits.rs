//! Time from arrival to first clinician contact, against the triage target (NEW-23).
//!
//! Arrival is the emergency encounter's `period_start`; first contact is `encounters.first_contact_at`,
//! recorded once by a named person. The report and the queue **only describe**: the order of care is a
//! clinician's decision, so the queue is a display (the same stance as NFR-11), and an attendance with no
//! triage category is counted as excluded rather than given a guessed one. The category used is the latest
//! assessment recorded.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use super::targets::{Standing, Targets, standing};
use super::{CATEGORIES, TriageService, colour};
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CategoryWaits {
    pub category: &'static str,
    #[schema(required = true)]
    pub colour: Option<&'static str>,
    pub target_minutes: i64,
    pub attended: u32,
    pub seen_within: u32,
    pub seen_past: u32,
    pub waiting_within: u32,
    pub waiting_past: u32,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct PastTarget {
    pub encounter_id: Uuid,
    pub category: String,
    pub arrived_at: DateTime<Utc>,
    #[schema(required = true)]
    pub first_contact_at: Option<DateTime<Utc>>,
    /// Minutes from arrival to first contact, or to now when not yet seen.
    pub minutes: i64,
    pub target_minutes: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(as = triage::WaitsReport)]
pub struct WaitsReport {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub attendances: u32,
    /// Attendances with no triage category: left out of every figure below, not guessed.
    pub excluded_without_category: u32,
    pub by_category: Vec<CategoryWaits>,
    pub past_target: Vec<PastTarget>,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[schema(as = triage::QueueEntry)]
pub struct QueueEntry {
    pub encounter_id: Uuid,
    /// `None` when not yet triaged; those are listed last.
    #[schema(required = true)]
    pub category: Option<String>,
    #[schema(required = true)]
    pub colour: Option<&'static str>,
    pub arrived_at: DateTime<Utc>,
    pub waiting_minutes: i64,
    #[schema(required = true)]
    pub target_minutes: Option<i64>,
    pub past_target: bool,
}

/// One emergency attendance as read from the database.
struct Attendance {
    encounter_id: Uuid,
    arrived_at: DateTime<Utc>,
    first_contact_at: Option<DateTime<Utc>>,
    category: Option<String>,
}

const LATEST_CATEGORY: &str = "(SELECT t.category FROM triage_assessments t WHERE t.encounter_id = e.id \
     ORDER BY t.assessed_at DESC, t.created_at DESC, t.id DESC LIMIT 1)";

fn minutes_between(a: DateTime<Utc>, b: DateTime<Utc>) -> i64 {
    (b - a).num_minutes()
}

/// Pure: build the report from attendances.
fn build_report(
    attendances: &[Attendance],
    targets: &Targets,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> WaitsReport {
    let mut by_category: Vec<CategoryWaits> = CATEGORIES
        .iter()
        .map(|c| CategoryWaits {
            category: c,
            colour: colour(c),
            target_minutes: targets.minutes(c).unwrap_or_default(),
            attended: 0,
            seen_within: 0,
            seen_past: 0,
            waiting_within: 0,
            waiting_past: 0,
        })
        .collect();
    let mut past_target = Vec::new();
    let mut excluded = 0;
    for a in attendances {
        let Some(cat) = a.category.as_deref() else {
            excluded += 1;
            continue;
        };
        let Some(i) = CATEGORIES.iter().position(|c| *c == cat) else {
            excluded += 1;
            continue;
        };
        let target = by_category[i].target_minutes;
        let s = standing(a.arrived_at, a.first_contact_at, now, target);
        let row = &mut by_category[i];
        row.attended += 1;
        match s {
            Standing::SeenWithin => row.seen_within += 1,
            Standing::SeenPast => row.seen_past += 1,
            Standing::WaitingWithin => row.waiting_within += 1,
            Standing::WaitingPast => row.waiting_past += 1,
        }
        if s.is_past() {
            past_target.push(PastTarget {
                encounter_id: a.encounter_id,
                category: cat.to_string(),
                arrived_at: a.arrived_at,
                first_contact_at: a.first_contact_at,
                minutes: minutes_between(a.arrived_at, a.first_contact_at.unwrap_or(now)),
                target_minutes: target,
            });
        }
    }
    past_target.sort_by_key(|p| (p.arrived_at, p.encounter_id));
    WaitsReport {
        from,
        to,
        attendances: attendances.len() as u32,
        excluded_without_category: excluded,
        by_category,
        past_target,
    }
}

/// Pure: the display order, most urgent category first then earliest arrival; not yet triaged last.
fn build_queue(
    attendances: &[Attendance],
    targets: &Targets,
    now: DateTime<Utc>,
) -> Vec<QueueEntry> {
    let mut entries: Vec<(usize, QueueEntry)> = attendances
        .iter()
        .filter(|a| a.first_contact_at.is_none())
        .map(|a| {
            let pos = a
                .category
                .as_deref()
                .and_then(|c| CATEGORIES.iter().position(|k| *k == c));
            let target = a.category.as_deref().and_then(|c| targets.minutes(c));
            let past = target.is_some_and(|t| standing(a.arrived_at, None, now, t).is_past());
            (
                pos.unwrap_or(CATEGORIES.len()),
                QueueEntry {
                    encounter_id: a.encounter_id,
                    category: pos.and(a.category.clone()),
                    colour: a.category.as_deref().and_then(colour),
                    arrived_at: a.arrived_at,
                    waiting_minutes: minutes_between(a.arrived_at, now),
                    target_minutes: target,
                    past_target: past,
                },
            )
        })
        .collect();
    entries.sort_by_key(|(pos, e)| (*pos, e.arrived_at, e.encounter_id));
    entries.into_iter().map(|(_, e)| e).collect()
}

impl TriageService {
    /// Record when a clinician first saw the patient. Once only, by a named person, not before arrival and
    /// not in the future; only on an emergency encounter that is not cancelled.
    pub async fn record_first_contact(
        &self,
        encounter_id: Uuid,
        seen_by: &str,
        seen_at: DateTime<Utc>,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            seen_by,
            "seen_by: the clinician who first saw the patient is required",
        )?;
        if by.chars().count() > super::MAX_TEXT || by.chars().any(char::is_control) {
            return Err(Error::validation(format!(
                "seen_by must be at most {} characters with no control characters",
                super::MAX_TEXT
            )));
        }
        if seen_at > now {
            return Err(Error::validation("seen_at cannot be in the future"));
        }
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let enc = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT class, status, period_start, first_contact_at FROM encounters \
                 WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
                [encounter_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("encounter {encounter_id}")))?;
        let class: String = enc.try_get("", "class").map_err(Error::Database)?;
        let status: String = enc.try_get("", "status").map_err(Error::Database)?;
        let arrived: DateTime<Utc> = enc.try_get("", "period_start").map_err(Error::Database)?;
        let already: Option<DateTime<Utc>> = enc
            .try_get("", "first_contact_at")
            .map_err(Error::Database)?;
        if class != "emergency" {
            return Err(Error::validation(
                "first clinician contact is only recorded on an emergency encounter",
            ));
        }
        if status == "cancelled" {
            return Err(Error::validation(
                "the encounter is cancelled; first contact cannot be recorded",
            ));
        }
        if already.is_some() {
            return Err(Error::conflict(
                "first clinician contact is already recorded for this encounter",
            ));
        }
        if seen_at < arrived {
            return Err(Error::validation("seen_at cannot be before arrival"));
        }
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE encounters SET first_contact_at = $2, first_contact_by = $3, updated_at = now() WHERE id = $1",
            [encounter_id.into(), seen_at.into(), by.into()],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "encounter",
            encounter_id,
            "record_first_contact",
            None,
            Some(serde_json::json!({
                "seen_by": by, "seen_at": seen_at, "arrived_at": arrived,
            })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(())
    }

    async fn attendances(&self, sql: &str, values: Vec<sea_orm::Value>) -> Result<Vec<Attendance>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                values,
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(Attendance {
                    encounter_id: r.try_get("", "id").map_err(Error::Database)?,
                    arrived_at: r.try_get("", "period_start").map_err(Error::Database)?,
                    first_contact_at: r.try_get("", "first_contact_at").map_err(Error::Database)?,
                    category: r.try_get("", "category").map_err(Error::Database)?,
                })
            })
            .collect()
    }

    /// Attendances that arrived in `[from, to)` against the targets. A window of more than 31 days or one
    /// that ends before it starts is refused.
    pub async fn waits_report(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        targets: &Targets,
        now: DateTime<Utc>,
    ) -> Result<WaitsReport> {
        if to <= from {
            return Err(Error::validation("to must be after from"));
        }
        if to - from > chrono::Duration::days(31) {
            return Err(Error::validation("the window can be at most 31 days"));
        }
        let sql = format!(
            "SELECT e.id, e.period_start, e.first_contact_at, {LATEST_CATEGORY} AS category \
             FROM encounters e WHERE e.class = 'emergency' AND e.status <> 'cancelled' \
               AND e.deleted_at IS NULL AND e.period_start >= $1 AND e.period_start < $2 \
             ORDER BY e.period_start, e.id"
        );
        let rows = self.attendances(&sql, vec![from.into(), to.into()]).await?;
        Ok(build_report(&rows, targets, from, to, now))
    }

    /// Everyone who has arrived and not yet been seen, in display order. The order of care is a clinician's.
    pub async fn queue(&self, targets: &Targets, now: DateTime<Utc>) -> Result<Vec<QueueEntry>> {
        let sql = format!(
            "SELECT e.id, e.period_start, e.first_contact_at, {LATEST_CATEGORY} AS category \
             FROM encounters e WHERE e.class = 'emergency' AND e.status IN ('arrived', 'in_progress') \
               AND e.deleted_at IS NULL AND e.first_contact_at IS NULL AND e.period_start <= $1 \
             ORDER BY e.period_start, e.id"
        );
        let rows = self.attendances(&sql, vec![now.into()]).await?;
        Ok(build_queue(&rows, targets, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap()
    }

    fn att(cat: Option<&str>, arrive_min: i64, seen_min: Option<i64>) -> Attendance {
        Attendance {
            encounter_id: Uuid::new_v4(),
            arrived_at: t0() + Duration::minutes(arrive_min),
            first_contact_at: seen_min.map(|m| t0() + Duration::minutes(m)),
            category: cat.map(str::to_string),
        }
    }

    #[test]
    fn the_report_lands_nine_ten_and_eleven_minutes_on_the_right_side() {
        let rows = vec![
            att(Some("very_urgent"), 0, Some(9)),
            att(Some("very_urgent"), 0, Some(10)),
            att(Some("very_urgent"), 0, Some(11)),
        ];
        let r = build_report(
            &rows,
            &Targets::default(),
            t0(),
            t0() + Duration::days(1),
            t0() + Duration::hours(1),
        );
        let vu = &r.by_category[1];
        assert_eq!((vu.seen_within, vu.seen_past), (2, 1));
        assert_eq!(r.past_target.len(), 1);
        assert_eq!(r.past_target[0].minutes, 11);
    }

    #[test]
    fn an_attendance_with_no_category_is_excluded_not_guessed() {
        let rows = vec![att(None, 0, Some(500)), att(Some("standard"), 0, Some(5))];
        let r = build_report(
            &rows,
            &Targets::default(),
            t0(),
            t0() + Duration::days(1),
            t0() + Duration::hours(10),
        );
        assert_eq!((r.attendances, r.excluded_without_category), (2, 1));
        assert_eq!(r.by_category.iter().map(|c| c.attended).sum::<u32>(), 1);
        assert!(r.past_target.is_empty());
    }

    #[test]
    fn a_configured_target_changes_the_verdict() {
        let rows = vec![att(Some("urgent"), 0, Some(90))];
        let report = |t: &Targets| {
            build_report(
                &rows,
                t,
                t0(),
                t0() + Duration::days(1),
                t0() + Duration::hours(3),
            )
        };
        assert_eq!(report(&Targets::default()).past_target.len(), 1);
        let lenient = Targets::from_json(r#"{"urgent": 120}"#).unwrap();
        assert!(report(&lenient).past_target.is_empty());
    }

    #[test]
    fn the_queue_is_category_then_arrival_with_untriaged_last_and_seen_left_out() {
        let rows = vec![
            att(None, 0, None),
            att(Some("standard"), 1, None),
            att(Some("very_urgent"), 5, None),
            att(Some("very_urgent"), 2, None),
            att(Some("immediate"), 9, Some(10)),
        ];
        let q = build_queue(&rows, &Targets::default(), t0() + Duration::minutes(30));
        let order: Vec<_> = q.iter().map(|e| e.category.as_deref()).collect();
        assert_eq!(
            order,
            vec![
                Some("very_urgent"),
                Some("very_urgent"),
                Some("standard"),
                None
            ]
        );
        assert!(q[0].arrived_at < q[1].arrived_at);
        assert!(q[0].past_target && !q[2].past_target);
        assert_eq!(q[3].target_minutes, None);
        assert!(!q[3].past_target);
    }
}
