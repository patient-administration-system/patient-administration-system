//! Clinic session templates and slot types (NEW-8).
//!
//! A template belongs to a schedule and is versioned. A version names a weekday, a start time (London local
//! time), a slot length and the run of slot types that make up one session, for example six `new`, four
//! `follow_up` and two `urgent` (the last two ring-fenced). **A version can generate nothing until a second,
//! named person approves it**: the approver must differ from the proposer. Generating slots for a range of
//! days is reproducible: the same version and the same days always make the same slots, and doing it again
//! adds none (a slot remembers its template and start time, and the pair is unique).
//!
//! **Ring-fencing** keeps a slot out of every *automatic* choice: a referral's first available slot, the
//! rebooking options, and IHE `$find`. A named person can still book it by choosing it, and can lift the
//! ring-fence. It is a hold for a purpose (urgent, a named pathway); it removes nothing from a patient who
//! asks for it (NFR-11), and no score ever sets it.
//!
//! What this is not: an overbooking planner (plan §7), and it does not rewrite slots already made when a new
//! version is approved: the new version applies to days generated after that.

pub mod report;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Europe::London;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::row::RowExt;
use crate::validation::named_person;
use crate::{Error, Result};

/// The slot types a template line may name (also a database CHECK).
pub const SLOT_TYPES: [&str; 4] = ["new", "follow_up", "urgent", "pifu"];
/// The most slots one session may hold.
pub const MAX_SLOTS_PER_SESSION: i32 = 100;
/// The longest range of days one generate call covers.
pub const MAX_GENERATE_DAYS: i64 = 92;

/// One run of slots of the same type in a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Line {
    pub slot_type: String,
    pub slots: i32,
    #[serde(default)]
    pub ring_fenced: bool,
}

/// What a person proposes as the next version.
#[derive(Debug, Clone)]
pub struct VersionInput {
    /// ISO weekday: 1 Monday to 7 Sunday.
    pub weekday: i16,
    /// Start of the first slot, London local time.
    pub start_time: NaiveTime,
    pub slot_minutes: i32,
    /// The first day the version applies to.
    pub effective_from: NaiveDate,
    pub lines: Vec<Line>,
    pub proposed_by: String,
}

/// One slot a session will make.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub slot_type: String,
    pub ring_fenced: bool,
}

/// Check a version's lines: at least one, known types, a positive count each, no more than a session holds.
pub fn validate_lines(lines: &[Line]) -> Result<()> {
    if lines.is_empty() {
        return Err(Error::validation("a session needs at least one line"));
    }
    let mut total = 0i32;
    for l in lines {
        if !SLOT_TYPES.contains(&l.slot_type.as_str()) {
            return Err(Error::validation(format!(
                "slot_type must be one of {}",
                SLOT_TYPES.join(", ")
            )));
        }
        if l.slots < 1 {
            return Err(Error::validation("each line needs at least one slot"));
        }
        total = total.saturating_add(l.slots);
    }
    if total > MAX_SLOTS_PER_SESSION {
        return Err(Error::validation(format!(
            "a session holds at most {MAX_SLOTS_PER_SESSION} slots"
        )));
    }
    Ok(())
}

/// The slots one session makes on `date`, or `None` when that local time does not exist that day (the clocks
/// go forward). An ambiguous time (the clocks go back) takes the earlier. Slots run back to back.
pub fn plan_session(
    date: NaiveDate,
    start_time: NaiveTime,
    slot_minutes: i32,
    lines: &[Line],
) -> Option<Vec<Planned>> {
    let first = London
        .from_local_datetime(&date.and_time(start_time))
        .earliest()?
        .with_timezone(&Utc);
    let mut out = Vec::new();
    let mut at = first;
    for l in lines {
        for _ in 0..l.slots {
            let end = at + Duration::minutes(i64::from(slot_minutes));
            out.push(Planned {
                start: at,
                end,
                slot_type: l.slot_type.clone(),
                ring_fenced: l.ring_fenced,
            });
            at = end;
        }
    }
    Some(out)
}

#[derive(Debug, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct VersionView {
    pub version: i32,
    pub weekday: i16,
    pub start_time: NaiveTime,
    pub slot_minutes: i32,
    pub effective_from: NaiveDate,
    pub lines: Vec<Line>,
    pub proposed_by: String,
    #[schema(required = true)]
    pub approved_by: Option<String>,
}

#[derive(Debug, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct TemplateView {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub name: String,
    pub versions: Vec<VersionView>,
}

#[derive(Debug, Default, Serialize, serde::Deserialize, utoipa::ToSchema, PartialEq)]
pub struct Generated {
    /// Slots made by this call.
    pub created: i64,
    /// Slots the template had already made for these days: left alone.
    pub already_there: i64,
    /// Days in the range on which no approved version applies.
    pub days_without_approved_version: i64,
    /// Days skipped because the session's start time does not exist that day (clocks going forward).
    pub days_skipped_clock_change: i64,
}

pub struct TemplateService {
    db: DatabaseConnection,
}

impl TemplateService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Start a template on a schedule. A named person; audited. It has no versions, so it makes no slots.
    pub async fn create(
        &self,
        schedule_id: Uuid,
        name: &str,
        created_by: &str,
        ctx: &UserContext,
    ) -> Result<Uuid> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 100 {
            return Err(Error::validation("name must be 1 to 100 characters"));
        }
        let by = named_person(
            created_by,
            "created_by: the person creating the template is required",
        )?;
        let txn = self.db.begin().await.map_err(Error::Database)?;
        txn.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT id FROM schedules WHERE id = $1",
            [schedule_id.into()],
        ))
        .await
        .map_err(Error::Database)?
        .ok_or_else(|| Error::not_found(format!("schedule {schedule_id}")))?;
        let id = Uuid::new_v4();
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO session_templates (id, schedule_id, name, created_by) VALUES ($1, $2, $3, $4)",
            [id.into(), schedule_id.into(), name.into(), by.into()],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "session_template",
            id,
            "create",
            None,
            Some(serde_json::json!({ "schedule_id": schedule_id, "name": name, "created_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(id)
    }

    /// Propose the next version of a template. It cannot generate anything until another person approves it.
    /// The first day it applies to cannot be in the past. Returns the new version number.
    pub async fn propose(
        &self,
        template_id: Uuid,
        v: &VersionInput,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<i32> {
        if !(1..=7).contains(&v.weekday) {
            return Err(Error::validation(
                "weekday must be 1 (Monday) to 7 (Sunday)",
            ));
        }
        if !(5..=240).contains(&v.slot_minutes) {
            return Err(Error::validation("slot_minutes must be between 5 and 240"));
        }
        validate_lines(&v.lines)?;
        let by = named_person(
            &v.proposed_by,
            "proposed_by: the person proposing is required",
        )?;
        if v.effective_from < now.with_timezone(&London).date_naive() {
            return Err(Error::validation("effective_from cannot be in the past"));
        }
        let txn = self.db.begin().await.map_err(Error::Database)?;
        // Serialise proposals and generation on the template.
        txn.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT id FROM session_templates WHERE id = $1 FOR UPDATE",
            [template_id.into()],
        ))
        .await
        .map_err(Error::Database)?
        .ok_or_else(|| Error::not_found(format!("session template {template_id}")))?;
        let next: i32 = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT (coalesce(max(version), 0) + 1)::integer AS n FROM session_template_versions \
                 WHERE template_id = $1",
                [template_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::internal("no row from max(version)"))?
            .col("n")?;
        let lines = serde_json::to_value(&v.lines)
            .map_err(|e| Error::internal(format!("lines could not be serialized: {e}")))?;
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO session_template_versions \
               (template_id, version, weekday, start_time, slot_minutes, effective_from, lines, proposed_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            [
                template_id.into(),
                next.into(),
                v.weekday.into(),
                v.start_time.into(),
                v.slot_minutes.into(),
                v.effective_from.into(),
                lines.clone().into(),
                by.into(),
            ],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "session_template",
            template_id,
            "propose_version",
            None,
            Some(serde_json::json!({
                "version": next, "weekday": v.weekday, "slot_minutes": v.slot_minutes,
                "effective_from": v.effective_from, "lines": lines, "proposed_by": by
            })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(next)
    }

    /// Approve a proposed version. A named person other than the proposer, once only; audited.
    pub async fn approve(
        &self,
        template_id: Uuid,
        version: i32,
        approved_by: &str,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(approved_by, "approved_by: the approving person is required")?;
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let row = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT proposed_by, approved_by FROM session_template_versions \
                 WHERE template_id = $1 AND version = $2 FOR UPDATE",
                [template_id.into(), version.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| {
                Error::not_found(format!("version {version} of template {template_id}"))
            })?;
        let proposer: String = row.col("proposed_by")?;
        let approved: Option<String> = row.col("approved_by")?;
        if approved.is_some() {
            return Err(Error::conflict("that version is already approved"));
        }
        if proposer.trim().to_lowercase() == by.to_lowercase() {
            return Err(Error::conflict(
                "a different person from the proposer must approve a version",
            ));
        }
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE session_template_versions SET approved_by = $3, approved_at = $4 \
             WHERE template_id = $1 AND version = $2",
            [template_id.into(), version.into(), by.into(), now.into()],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "session_template",
            template_id,
            "approve_version",
            None,
            Some(serde_json::json!({ "version": version, "approved_by": by, "proposed_by": proposer })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// Templates (optionally of one schedule) with their versions.
    pub async fn list(&self, schedule_id: Option<Uuid>) -> Result<Vec<TemplateView>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, schedule_id, name FROM session_templates \
                 WHERE ($1::uuid IS NULL OR schedule_id = $1) ORDER BY created_at, id LIMIT 200",
                [schedule_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            let id: Uuid = r.col("id")?;
            out.push(TemplateView {
                id,
                schedule_id: r.col("schedule_id")?,
                name: r.col("name")?,
                versions: self.versions(id).await?,
            });
        }
        Ok(out)
    }

    async fn versions(&self, template_id: Uuid) -> Result<Vec<VersionView>> {
        self.db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT version, weekday, start_time, slot_minutes, effective_from, lines, proposed_by, \
                        approved_by FROM session_template_versions WHERE template_id = $1 ORDER BY version",
                [template_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .iter()
            .map(|r| {
                let lines: serde_json::Value = r.col("lines")?;
                Ok(VersionView {
                    version: r.col("version")?,
                    weekday: r.col("weekday")?,
                    start_time: r.col("start_time")?,
                    slot_minutes: r.col("slot_minutes")?,
                    effective_from: r.col("effective_from")?,
                    lines: serde_json::from_value(lines)
                        .map_err(|e| Error::internal(format!("stored lines unreadable: {e}")))?,
                    proposed_by: r.col("proposed_by")?,
                    approved_by: r.col("approved_by")?,
                })
            })
            .collect()
    }

    /// Make the slots for every day in `[from, to]`, from the approved version in force on that day (the
    /// highest approved version whose `effective_from` is on or before it). Days that are not that version's
    /// weekday make nothing. Reproducible and idempotent. Not before today, and at most 92 days at once.
    pub async fn generate(
        &self,
        template_id: Uuid,
        from: NaiveDate,
        to: NaiveDate,
        generated_by: &str,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<Generated> {
        let by = named_person(
            generated_by,
            "generated_by: the person generating slots is required",
        )?;
        if to < from {
            return Err(Error::validation("to cannot be before from"));
        }
        if (to - from).num_days() + 1 > MAX_GENERATE_DAYS {
            return Err(Error::validation(format!(
                "generate at most {MAX_GENERATE_DAYS} days at a time"
            )));
        }
        if from < now.with_timezone(&London).date_naive() {
            return Err(Error::validation(
                "slots cannot be generated for a day in the past",
            ));
        }
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let schedule_id: Uuid = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT schedule_id FROM session_templates WHERE id = $1 FOR UPDATE",
                [template_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("session template {template_id}")))?
            .col("schedule_id")?;
        // Approved versions, oldest first, so the last that applies on a day wins.
        let versions = txn
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT version, weekday, start_time, slot_minutes, effective_from, lines \
                 FROM session_template_versions WHERE template_id = $1 AND approved_by IS NOT NULL \
                 ORDER BY effective_from, version",
                [template_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        struct V {
            version: i32,
            weekday: i16,
            start_time: NaiveTime,
            slot_minutes: i32,
            effective_from: NaiveDate,
            lines: Vec<Line>,
        }
        let versions = versions
            .iter()
            .map(|r| {
                let lines: serde_json::Value = r.col("lines")?;
                Ok(V {
                    version: r.col("version")?,
                    weekday: r.col("weekday")?,
                    start_time: r.col("start_time")?,
                    slot_minutes: r.col("slot_minutes")?,
                    effective_from: r.col("effective_from")?,
                    lines: serde_json::from_value(lines)
                        .map_err(|e| Error::internal(format!("stored lines unreadable: {e}")))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut out = Generated::default();
        let mut day = from;
        while day <= to {
            // The latest-effective approved version on or before the day.
            match versions.iter().rfind(|v| v.effective_from <= day) {
                None => out.days_without_approved_version += 1,
                Some(v)
                    if i16::try_from(day.weekday().number_from_monday()).ok()
                        == Some(v.weekday) =>
                {
                    match plan_session(day, v.start_time, v.slot_minutes, &v.lines) {
                        None => out.days_skipped_clock_change += 1,
                        Some(plan) => {
                            for p in plan {
                                let n = txn
                                    .execute_raw(Statement::from_sql_and_values(
                                        DbBackend::Postgres,
                                        "INSERT INTO slots (id, schedule_id, start_datetime, end_datetime, \
                                           status, slot_type, ring_fenced, template_id, template_version, \
                                           created_at, updated_at) \
                                         VALUES ($1, $2, $3, $4, 'free', $5, $6, $7, $8, now(), now()) \
                                         ON CONFLICT (template_id, start_datetime) \
                                           WHERE template_id IS NOT NULL DO NOTHING",
                                        [
                                            Uuid::new_v4().into(),
                                            schedule_id.into(),
                                            p.start.into(),
                                            p.end.into(),
                                            p.slot_type.into(),
                                            p.ring_fenced.into(),
                                            template_id.into(),
                                            v.version.into(),
                                        ],
                                    ))
                                    .await
                                    .map_err(Error::Database)?
                                    .rows_affected();
                                if n == 1 {
                                    out.created += 1;
                                } else {
                                    out.already_there += 1;
                                }
                            }
                        }
                    }
                }
                Some(_) => {}
            }
            day += Duration::days(1);
        }
        AuditLogRepository::log(
            &txn,
            "session_template",
            template_id,
            "generate_slots",
            None,
            Some(serde_json::json!({
                "from": from, "to": to, "generated_by": by,
                "created": out.created, "already_there": out.already_there
            })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(out)
    }

    /// Ring-fence a free slot still to come, or lift the ring-fence. A named person; audited.
    pub async fn set_ring_fence(
        &self,
        slot_id: Uuid,
        ring_fenced: bool,
        set_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            set_by,
            "set_by: the person changing the ring-fence is required",
        )?;
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let row = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT ring_fenced, status, start_datetime > now() AS future FROM slots \
                 WHERE id = $1 FOR UPDATE",
                [slot_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("slot {slot_id}")))?;
        let old: bool = row.col("ring_fenced")?;
        let status: String = row.col("status")?;
        let future: bool = row.col("future")?;
        if status != "free" || !future {
            return Err(Error::conflict(
                "only a free slot still to come can be ring-fenced or released",
            ));
        }
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE slots SET ring_fenced = $2, updated_at = now() WHERE id = $1",
            [slot_id.into(), ring_fenced.into()],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "slot",
            slot_id,
            "set_ring_fence",
            Some(serde_json::json!({ "ring_fenced": old })),
            Some(serde_json::json!({ "ring_fenced": ring_fenced, "set_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> Vec<Line> {
        vec![
            Line {
                slot_type: "new".into(),
                slots: 2,
                ring_fenced: false,
            },
            Line {
                slot_type: "urgent".into(),
                slots: 1,
                ring_fenced: true,
            },
        ]
    }

    #[test]
    fn a_session_runs_back_to_back_in_the_stated_order() {
        let day = NaiveDate::from_ymd_opt(2027, 1, 12).unwrap(); // a winter Tuesday: London is UTC
        let p = plan_session(day, NaiveTime::from_hms_opt(9, 0, 0).unwrap(), 20, &lines()).unwrap();
        let starts: Vec<_> = p
            .iter()
            .map(|s| s.start.format("%H:%M").to_string())
            .collect();
        assert_eq!(starts, ["09:00", "09:20", "09:40"]);
        assert_eq!(p[2].end.format("%H:%M").to_string(), "10:00");
        assert_eq!(p[2].slot_type, "urgent");
        assert!(p[2].ring_fenced && !p[0].ring_fenced);
    }

    #[test]
    fn summer_time_is_an_hour_ahead_of_utc_and_the_missing_hour_is_skipped() {
        let summer = NaiveDate::from_ymd_opt(2027, 7, 13).unwrap();
        let p = plan_session(
            summer,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
            &lines(),
        )
        .unwrap();
        assert_eq!(p[0].start.format("%H:%M").to_string(), "08:00");
        // 2027-03-28 01:30 London does not exist (the clocks go forward at 01:00).
        let gap = NaiveDate::from_ymd_opt(2027, 3, 28).unwrap();
        assert!(
            plan_session(
                gap,
                NaiveTime::from_hms_opt(1, 30, 0).unwrap(),
                30,
                &lines()
            )
            .is_none()
        );
    }

    #[test]
    fn the_same_inputs_always_plan_the_same_session() {
        let day = NaiveDate::from_ymd_opt(2027, 1, 12).unwrap();
        let t = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
        assert_eq!(
            plan_session(day, t, 15, &lines()),
            plan_session(day, t, 15, &lines())
        );
    }

    #[test]
    fn lines_are_checked() {
        assert!(validate_lines(&[]).is_err());
        assert!(
            validate_lines(&[Line {
                slot_type: "routine".into(),
                slots: 1,
                ring_fenced: false
            }])
            .is_err()
        );
        assert!(
            validate_lines(&[Line {
                slot_type: "new".into(),
                slots: 0,
                ring_fenced: false
            }])
            .is_err()
        );
        assert!(
            validate_lines(&[Line {
                slot_type: "new".into(),
                slots: 101,
                ring_fenced: false
            }])
            .is_err()
        );
        assert!(validate_lines(&lines()).is_ok());
    }
}
