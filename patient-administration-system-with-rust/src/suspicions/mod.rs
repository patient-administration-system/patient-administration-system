//! Suspected conditions: tracking suspicion during referrals (WPAS-1, WPAS-8, spec 20, FR-207 to FR-221).
//!
//! A referral is often made because a clinician *suspects* a condition: cancer, sleep apnoea, diabetes,
//! cardiovascular disease, Parkinson's, dementia. The wait that matters starts at that point of suspicion. This
//! module records the suspicion and keeps its wait visible until its clock stops.
//!
//! - **Which conditions, and their rules, are configuration** ([`profile`]): each condition's subtypes, its
//!   target, whether a patient's own cancel is held for a person, and the booking priority it raises a referral
//!   to. The rules are copied onto the suspicion when it is recorded, so a later change of profile never
//!   rewrites a wait already counting (FR-221).
//! - **The PAS decides nothing.** Staff record what a named clinician suspected and decided (`suspected_by`,
//!   `stopped_by`); the module never suspects, downgrades or ranks.
//! - **The clock never pauses** (FR-208), whatever the profile. Days waited are whole days from the point of
//!   suspicion to the stop date, or to today in the service's time zone (`PAS_TIMEZONE`) while open.
//! - **It stops once** (FR-209), for `treatment_started`, `downgraded`, `died` or `entered_in_error`.
//! - **No invented targets** (FR-211): a condition has a target only when its profile names one and its source.

pub mod profile;

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Row, Stmt, Transact};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::db::repositories::waitlist::{priority_from_str, priority_to_str};
use crate::models::waitlist::Priority;
use crate::{Error, Result};
pub use profile::Profile;

crate::string_enum! {
    /// Why a suspicion's clock stopped. Any one stops it, and only one is recorded.
    pub enum StopReason {
        /// The start of first definitive treatment.
        TreatmentStarted = "treatment_started",
        /// The suspicion was ruled out (for cancer, a downgrade).
        Downgraded = "downgraded",
        Died = "died",
        /// Recorded by mistake: kept for the audit, left out of every count.
        EnteredInError = "entered_in_error",
    }
}

crate::string_enum! {
    /// Where a suspicion's wait stands against its target. Computed, never stored.
    pub enum TargetStatus {
        /// The condition has no target (FR-211).
        NoTarget = "no_target",
        Within = "within",
        /// Open, and within [`NEAR_TARGET_DAYS`] of the target: a display aid, not a rule.
        Near = "near",
        Past = "past",
    }
}

/// How close to a target an open wait is marked `near` (FR-212).
pub const NEAR_TARGET_DAYS: i64 = 7;

/// Today in the service's time zone, so "days waited" turns over at local midnight.
pub fn today() -> NaiveDate {
    Utc::now()
        .with_timezone(&crate::inbound::messages::timezone())
        .date_naive()
}

/// Whole days waited and where they stand against a target (FR-208, FR-211, FR-212).
pub fn wait(
    target: Option<i64>,
    suspected_on: NaiveDate,
    stopped_on: Option<NaiveDate>,
    today: NaiveDate,
) -> (i64, TargetStatus) {
    let days = (stopped_on.unwrap_or(today) - suspected_on)
        .num_days()
        .max(0);
    let status = match target {
        None => TargetStatus::NoTarget,
        Some(t) if days > t => TargetStatus::Past,
        Some(t) if stopped_on.is_none() && days > t - NEAR_TARGET_DAYS => TargetStatus::Near,
        Some(_) => TargetStatus::Within,
    };
    (days, status)
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Suspicion {
    pub id: Uuid,
    pub referral_id: Uuid,
    pub person_id: Uuid,
    /// The profile it was recorded under.
    pub profile: String,
    /// A condition code of that profile.
    pub condition: String,
    /// The condition's name in the active profile; none when it was recorded under another.
    #[schema(required = true)]
    pub condition_name: Option<String>,
    #[schema(required = true)]
    pub subtype: Option<String>,
    /// The list the subtype comes from.
    #[schema(required = true)]
    pub subtype_scheme: Option<String>,
    #[schema(required = true)]
    pub subtype_name: Option<String>,
    #[schema(required = true)]
    pub unique_pathway_identifier: Option<String>,
    /// The point of suspicion: the clock's start.
    pub suspected_on: NaiveDate,
    /// The clinician who suspected it.
    pub suspected_by: String,
    pub recorded_by: String,
    pub recorded_at: DateTime<Utc>,
    #[schema(required = true)]
    pub stop_reason: Option<StopReason>,
    #[schema(required = true)]
    pub stopped_on: Option<NaiveDate>,
    #[schema(required = true)]
    pub stopped_by: Option<String>,
    /// Whole days from the point of suspicion to the stop date, or to today while open. Never paused.
    pub days_waited: i64,
    /// The target recorded with it, or none (FR-211, FR-221).
    #[schema(required = true)]
    pub target_days: Option<i64>,
    pub target_status: TargetStatus,
    /// Whether a patient's own cancel is held for a person while it is open (FR-214).
    pub hold_patient_cancel: bool,
    /// The waiting-list priority it raises its referral to (FR-220).
    #[schema(required = true)]
    pub booking_priority: Option<Priority>,
    pub updated_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, referral_id, person_id, profile, condition, subtype, subtype_scheme, \
     unique_pathway_identifier, suspected_on, suspected_by, recorded_by, recorded_at, stop_reason, stopped_on, \
     stopped_by, target_days, hold_patient_cancel, booking_priority, updated_at";

fn from_row(r: &Row, active: &Profile, today: NaiveDate) -> Result<Suspicion> {
    let profile: String = r.col("profile")?;
    let condition: String = r.col("condition")?;
    let subtype: Option<String> = r.col("subtype")?;
    let subtype_scheme: Option<String> = r.col("subtype_scheme")?;
    let suspected_on: NaiveDate = r.col("suspected_on")?;
    let stopped_on: Option<NaiveDate> = r.col("stopped_on")?;
    let target_days: Option<i32> = r.col("target_days")?;
    let target_days = target_days.map(i64::from);
    let (days_waited, target_status) = wait(target_days, suspected_on, stopped_on, today);
    // Names come from the active profile, and only for what it defines: a row recorded under another profile
    // keeps its codes, which are never relabelled with words they did not mean.
    let named = (profile == active.name)
        .then(|| active.condition(&condition))
        .flatten();
    let subtype_name = match (named, &subtype, &subtype_scheme) {
        (Some(c), Some(code), Some(scheme))
            if c.subtypes.as_ref().is_some_and(|l| &l.scheme == scheme) =>
        {
            c.subtype_name(code).map(String::from)
        }
        _ => None,
    };
    Ok(Suspicion {
        id: r.col("id")?,
        referral_id: r.col("referral_id")?,
        person_id: r.col("person_id")?,
        condition_name: named.map(|c| c.name.clone()),
        profile,
        condition,
        subtype,
        subtype_scheme,
        subtype_name,
        unique_pathway_identifier: r.col("unique_pathway_identifier")?,
        suspected_on,
        suspected_by: r.col("suspected_by")?,
        recorded_by: r.col("recorded_by")?,
        recorded_at: r.col("recorded_at")?,
        stop_reason: r
            .col::<Option<String>>("stop_reason")?
            .map(|s| s.parse())
            .transpose()?,
        stopped_on,
        stopped_by: r.col("stopped_by")?,
        days_waited,
        target_days,
        target_status,
        hold_patient_cancel: r.col("hold_patient_cancel")?,
        booking_priority: r
            .col::<Option<String>>("booking_priority")?
            .map(|s| priority_from_str(&s))
            .transpose()?,
        updated_at: r.col("updated_at")?,
    })
}

/// What a person records: the condition, the point of suspicion and who suspected it.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct NewSuspicion {
    /// A condition code of the active profile (`GET /api/suspicion-profile`).
    pub condition: String,
    /// A code from the condition's subtype list, if it has one; refused if it has none.
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub unique_pathway_identifier: Option<String>,
    pub suspected_on: NaiveDate,
    pub suspected_by: String,
    pub recorded_by: String,
}

/// One row of the report (FR-213): suspicions whose clock stopped in the period.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReportRow {
    pub profile: String,
    pub condition: String,
    #[schema(required = true)]
    pub condition_name: Option<String>,
    #[schema(required = true)]
    pub subtype: Option<String>,
    #[schema(required = true)]
    pub subtype_scheme: Option<String>,
    /// The target they were recorded with; rows with different targets are counted apart.
    #[schema(required = true)]
    pub target_days: Option<i64>,
    pub stopped: i64,
    /// None when there is no target.
    #[schema(required = true)]
    pub within_target: Option<i64>,
    #[schema(required = true)]
    pub share_within_target: Option<f64>,
    #[schema(required = true)]
    pub median_days: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SuspicionReport {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub rows: Vec<ReportRow>,
}

/// Is there an open suspicion whose condition holds a patient's cancel, on the referral this appointment was
/// booked from? (FR-214) Read from the rules recorded on the suspicion, so no profile is needed here.
pub async fn appointment_is_held_by_a_suspicion<C: Conn>(
    conn: &C,
    appointment_id: Uuid,
) -> Result<bool> {
    Ok(conn
        .one(Stmt::new(
            "SELECT 1 AS hit FROM appointments a JOIN suspected_conditions s ON s.referral_id = a.referral_id \
             WHERE a.id = $1 AND s.hold_patient_cancel AND s.stop_reason IS NULL LIMIT 1",
            [appointment_id.into()],
        ))
        .await?
        .is_some())
}

/// The highest booking priority among a referral's open suspicions, if any sets one (FR-220).
pub async fn booking_priority_of_referral<C: Conn>(
    conn: &C,
    referral_id: Uuid,
) -> Result<Option<Priority>> {
    let rows = conn
        .all(Stmt::new(
            "SELECT booking_priority FROM suspected_conditions \
             WHERE referral_id = $1 AND stop_reason IS NULL AND booking_priority IS NOT NULL",
            [referral_id.into()],
        ))
        .await?;
    let mut best: Option<Priority> = None;
    for r in &rows {
        let p = priority_from_str(&r.col::<String>("booking_priority")?)?;
        best = best.max(Some(p));
    }
    Ok(best)
}

pub struct SuspicionService {
    db: Pool,
    profile: Arc<Profile>,
}

impl SuspicionService {
    pub fn new(db: Pool, profile: Arc<Profile>) -> Self {
        Self { db, profile }
    }

    /// The active profile (`GET /api/suspicion-profile`).
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub async fn get(&self, id: Uuid) -> Result<Suspicion> {
        let r = self
            .db
            .one(Stmt::new(
                format!("SELECT {COLUMNS} FROM suspected_conditions WHERE id = $1"),
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("suspected condition {id}")))?;
        from_row(&r, &self.profile, today())
    }

    /// Every suspicion on a referral, oldest point of suspicion first.
    pub async fn for_referral(&self, referral_id: Uuid) -> Result<Vec<Suspicion>> {
        self.list(
            "WHERE referral_id = $1 ORDER BY suspected_on, id",
            vec![referral_id.into()],
        )
        .await
    }

    /// Every suspicion of a person, oldest point of suspicion first.
    pub async fn for_person(&self, person_id: Uuid) -> Result<Vec<Suspicion>> {
        self.list(
            "WHERE person_id = $1 ORDER BY suspected_on, id",
            vec![person_id.into()],
        )
        .await
    }

    async fn list(&self, tail: &str, params: Vec<crate::db::sql::Param>) -> Result<Vec<Suspicion>> {
        let today = today();
        let rows = self
            .db
            .all(Stmt::new(
                format!("SELECT {COLUMNS} FROM suspected_conditions {tail}"),
                params,
            ))
            .await?;
        rows.iter()
            .map(|r| from_row(r, &self.profile, today))
            .collect()
    }

    /// The open list (FR-212): every open suspicion, longest wait first, optionally of one condition or subtype.
    pub async fn open(
        &self,
        condition: Option<&str>,
        subtype: Option<&str>,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<Suspicion>> {
        let rows = rows.into();
        self.list(
            "WHERE stop_reason IS NULL AND ($1::text IS NULL OR condition = $1) \
             AND ($2::text IS NULL OR subtype = $2) ORDER BY suspected_on, id LIMIT $3 OFFSET $4",
            vec![
                condition.map(String::from).into(),
                subtype.map(String::from).into(),
                rows.fetch().into(),
                rows.offset().into(),
            ],
        )
        .await
    }

    /// Record a suspicion on a referral under the active profile's rules (FR-207, FR-210, FR-215, FR-220,
    /// FR-221).
    pub async fn record(
        &self,
        referral_id: Uuid,
        n: NewSuspicion,
        ctx: &UserContext,
    ) -> Result<Suspicion> {
        let condition = self.profile.condition(n.condition.trim()).ok_or_else(|| {
            Error::validation(format!(
                "condition must be one of {} (profile {})",
                self.profile.condition_codes().join(", "),
                self.profile.name
            ))
        })?;
        let (subtype, subtype_scheme) = self.subtype_for(condition, n.subtype.as_deref())?;
        let upi = n
            .unique_pathway_identifier
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty());
        if upi.as_ref().is_some_and(|u| u.chars().count() > 24) {
            return Err(Error::validation(
                "unique_pathway_identifier is at most 24 characters",
            ));
        }
        if n.suspected_on > today() {
            return Err(Error::validation(
                "suspected_on (the point of suspicion) cannot be in the future",
            ));
        }
        let suspected_by = crate::validation::named_person(
            &n.suspected_by,
            "a suspicion names the clinician who suspected it (suspected_by)",
        )?
        .to_string();
        let recorded_by = crate::validation::named_person(
            &n.recorded_by,
            "a suspicion names who recorded it (recorded_by)",
        )?
        .to_string();
        let id = Uuid::new_v4();
        let ctx = ctx.clone();
        let profile_name = self.profile.name.clone();
        let code = condition.code.clone();
        let target = condition.target_days();
        let hold = condition.hold_patient_cancel;
        let priority = condition.booking_priority;
        self.db
            .in_txn::<_, ()>(|txn| {
                Box::pin(async move {
                    let person_id: Uuid = txn
                        .one(Stmt::new(
                            "SELECT person_id FROM referrals WHERE id = $1",
                            [referral_id.into()],
                        ))
                        .await?
                        .ok_or_else(|| Error::not_found(format!("referral {referral_id}")))?
                        .col("person_id")?;
                    let open = txn
                        .one(Stmt::new(
                            "SELECT 1 AS hit FROM suspected_conditions \
                             WHERE referral_id = $1 AND condition = $2 AND stop_reason IS NULL",
                            [referral_id.into(), code.clone().into()],
                        ))
                        .await?;
                    if open.is_some() {
                        return Err(Error::conflict(format!(
                            "that referral already has an open suspicion of {code}"
                        )));
                    }
                    txn.run(Stmt::new(
                        "INSERT INTO suspected_conditions (id, referral_id, person_id, profile, condition, subtype, \
                           subtype_scheme, unique_pathway_identifier, suspected_on, suspected_by, recorded_by, \
                           target_days, hold_patient_cancel, booking_priority) \
                         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
                        [
                            id.into(),
                            referral_id.into(),
                            person_id.into(),
                            profile_name.clone().into(),
                            code.clone().into(),
                            subtype.clone().into(),
                            subtype_scheme.clone().into(),
                            upi.clone().into(),
                            n.suspected_on.into(),
                            suspected_by.clone().into(),
                            recorded_by.clone().into(),
                            target.map(|t| t as i32).into(),
                            hold.into(),
                            priority.map(|p| priority_to_str(p).to_string()).into(),
                        ],
                    ))
                    .await?;
                    let detail = serde_json::json!({
                        "suspicion_id": id,
                        "referral_id": referral_id,
                        "person_id": person_id,
                        "profile": profile_name,
                        "condition": code,
                        "subtype": subtype,
                        "subtype_scheme": subtype_scheme,
                        "suspected_on": n.suspected_on,
                        "suspected_by": suspected_by,
                        "recorded_by": recorded_by,
                        "target_days": target,
                    });
                    AuditLogRepository::log(
                        txn,
                        "suspected_condition",
                        id,
                        "recorded",
                        None,
                        Some(detail.clone()),
                        &ctx,
                    )
                    .await?;
                    OutboxRepository::publish(txn, "SuspicionRecorded", &detail).await?;
                    if let Some(p) = priority {
                        raise_waiting_entries(txn, referral_id, p, id, &ctx).await?;
                    }
                    Ok(())
                })
            })
            .await?;
        self.get(id).await
    }

    /// The subtype and its list's scheme, checked against the condition's list (FR-210).
    fn subtype_for(
        &self,
        condition: &profile::Condition,
        subtype: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        let subtype = subtype.map(str::trim).filter(|s| !s.is_empty());
        let codes = |l: &profile::SubtypeList| {
            l.codes
                .iter()
                .map(|s| s.code.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (subtype, subtype_scheme): (Option<String>, Option<String>) =
            match (&condition.subtypes, subtype) {
                (None, Some(_)) => {
                    return Err(Error::validation(format!(
                        "{} has no subtypes in profile {}",
                        condition.code, self.profile.name
                    )));
                }
                (None, None) => (None, None),
                (Some(list), None) if list.required => {
                    return Err(Error::validation(format!(
                        "a suspicion of {} needs a subtype from {} ({})",
                        condition.code,
                        list.title,
                        codes(list)
                    )));
                }
                (Some(_), None) => (None, None),
                (Some(list), Some(code)) => {
                    if condition.subtype_name(code).is_none() {
                        return Err(Error::validation(format!(
                            "subtype must be one of {}",
                            codes(list)
                        )));
                    }
                    (Some(code.to_string()), Some(list.scheme.clone()))
                }
            };
        Ok((subtype, subtype_scheme))
    }

    /// Stop a suspicion's clock, once (FR-209, FR-215), if it is still the version the caller read (FR-216).
    pub async fn stop_if_unchanged(
        &self,
        id: Uuid,
        reason: &str,
        stopped_on: NaiveDate,
        stopped_by: &str,
        expected: Option<DateTime<Utc>>,
        ctx: &UserContext,
    ) -> Result<Suspicion> {
        let reason: StopReason = reason.parse().map_err(|_| {
            Error::validation(format!(
                "reason must be one of {}",
                StopReason::NAMES.join(", ")
            ))
        })?;
        if stopped_on > today() {
            return Err(Error::validation("stopped_on cannot be in the future"));
        }
        let stopped_by = crate::validation::named_person(
            stopped_by,
            "stopping a suspicion's clock names who recorded it (stopped_by)",
        )?
        .to_string();
        let ctx = ctx.clone();
        self.db
            .in_txn::<_, ()>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .one(Stmt::new(
                            "SELECT person_id, referral_id, condition, suspected_on, stop_reason, updated_at \
                             FROM suspected_conditions WHERE id = $1 FOR UPDATE",
                            [id.into()],
                        ))
                        .await?
                        .ok_or_else(|| Error::not_found(format!("suspected condition {id}")))?;
                    let stored: DateTime<Utc> = r.col("updated_at")?;
                    if !crate::api::rest::concurrency::is_current(stored, expected) {
                        return Err(Error::precondition_failed(
                            "that suspicion has changed since you read it; read it again",
                        ));
                    }
                    if r.col::<Option<String>>("stop_reason")?.is_some() {
                        return Err(Error::conflict(
                            "that suspicion's clock has already stopped",
                        ));
                    }
                    let suspected_on: NaiveDate = r.col("suspected_on")?;
                    if stopped_on < suspected_on {
                        return Err(Error::validation(
                            "stopped_on cannot be before the point of suspicion",
                        ));
                    }
                    txn.run(Stmt::new(
                        "UPDATE suspected_conditions SET stop_reason = $2, stopped_on = $3, stopped_by = $4, \
                           stopped_at = now(), updated_at = now() WHERE id = $1",
                        [
                            id.into(),
                            reason.as_str().into(),
                            stopped_on.into(),
                            stopped_by.clone().into(),
                        ],
                    ))
                    .await?;
                    let detail = serde_json::json!({
                        "suspicion_id": id,
                        "referral_id": r.col::<Uuid>("referral_id")?,
                        "person_id": r.col::<Uuid>("person_id")?,
                        "condition": r.col::<String>("condition")?,
                        "reason": reason.as_str(),
                        "stopped_on": stopped_on,
                        "stopped_by": stopped_by,
                    });
                    AuditLogRepository::log(
                        txn,
                        "suspected_condition",
                        id,
                        "stopped",
                        None,
                        Some(detail.clone()),
                        &ctx,
                    )
                    .await?;
                    OutboxRepository::publish(txn, "SuspicionStopped", &detail).await?;
                    Ok(())
                })
            })
            .await?;
        self.get(id).await
    }

    /// The report (FR-213): suspicions whose clock stopped from `from` to `to` inclusive, each once, per
    /// profile, condition, subtype and recorded target. `entered_in_error` is left out.
    pub async fn report(&self, from: NaiveDate, to: NaiveDate) -> Result<SuspicionReport> {
        if to < from {
            return Err(Error::validation("`from` must not be after `to`"));
        }
        let rows = self
            .db
            .all(Stmt::new(
                "SELECT profile, condition, subtype, subtype_scheme, target_days, suspected_on, stopped_on \
                 FROM suspected_conditions \
                 WHERE stop_reason IS NOT NULL AND stop_reason <> 'entered_in_error' \
                   AND stopped_on BETWEEN $1 AND $2",
                [from.into(), to.into()],
            ))
            .await?;
        type Key = (String, String, Option<String>, Option<String>, Option<i32>);
        let mut groups: std::collections::BTreeMap<Key, Vec<i64>> =
            std::collections::BTreeMap::new();
        for r in &rows {
            let start: NaiveDate = r.col("suspected_on")?;
            let stop: NaiveDate = r.col("stopped_on")?;
            groups
                .entry((
                    r.col("profile")?,
                    r.col("condition")?,
                    r.col("subtype_scheme")?,
                    r.col("subtype")?,
                    r.col("target_days")?,
                ))
                .or_default()
                .push((stop - start).num_days());
        }
        let mut out = Vec::new();
        for ((profile, condition, scheme, subtype, target), mut days) in groups {
            let target = target.map(i64::from);
            days.sort_unstable();
            let n = days.len() as i64;
            let within = target.map(|t| days.iter().filter(|d| **d <= t).count() as i64);
            let condition_name = (profile == self.profile.name)
                .then(|| self.profile.condition(&condition).map(|c| c.name.clone()))
                .flatten();
            out.push(ReportRow {
                profile,
                condition,
                condition_name,
                subtype,
                subtype_scheme: scheme,
                target_days: target,
                stopped: n,
                within_target: within,
                share_within_target: within.map(|w| w as f64 / n as f64),
                median_days: median(&days),
            });
        }
        Ok(SuspicionReport {
            from,
            to,
            rows: out,
        })
    }
}

/// Raise a referral's waiting entries to `to`, never lower, with an audit row for each (FR-220).
pub async fn raise_waiting_entries<C: Conn>(
    conn: &C,
    referral_id: Uuid,
    to: Priority,
    suspicion_id: Uuid,
    ctx: &UserContext,
) -> Result<()> {
    let rows = conn
        .all(Stmt::new(
            "SELECT id, priority FROM waitlist_entries WHERE referral_id = $1 AND status = 'waiting' FOR UPDATE",
            [referral_id.into()],
        ))
        .await?;
    for r in &rows {
        let entry: Uuid = r.col("id")?;
        let from = priority_from_str(&r.col::<String>("priority")?)?;
        if from >= to {
            continue;
        }
        conn.run(Stmt::new(
            "UPDATE waitlist_entries SET priority = $2, updated_at = now() WHERE id = $1",
            [entry.into(), priority_to_str(to).into()],
        ))
        .await?;
        AuditLogRepository::log(
            conn,
            "waitlist_entry",
            entry,
            "priority_raised",
            None,
            Some(serde_json::json!({
                "from": priority_to_str(from),
                "to": priority_to_str(to),
                "suspicion_id": suspicion_id,
            })),
            ctx,
        )
        .await?;
    }
    Ok(())
}

/// The median of sorted whole days, or none for an empty list.
fn median(sorted: &[i64]) -> Option<f64> {
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2] as f64),
        _ => Some((sorted[n / 2 - 1] + sorted[n / 2]) as f64 / 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn test_the_wait_is_real_days_and_marks_near_and_past() {
        let start = d("2026-01-01");
        let t = Some(62);
        assert_eq!(wait(t, start, None, d("2026-01-01")).0, 0);
        assert_eq!(
            wait(t, start, None, d("2026-02-25")).1,
            TargetStatus::Within,
            "day 55"
        );
        assert_eq!(
            wait(t, start, None, d("2026-02-26")).1,
            TargetStatus::Near,
            "day 56"
        );
        assert_eq!(
            wait(t, start, None, d("2026-03-04")).1,
            TargetStatus::Near,
            "day 62 is within, but near"
        );
        assert_eq!(
            wait(t, start, None, d("2026-03-05")).1,
            TargetStatus::Past,
            "day 63"
        );
        // A stopped clock is judged on its stop date, and is never "near".
        assert_eq!(
            wait(t, start, Some(d("2026-03-01")), d("2026-12-01")),
            (59, TargetStatus::Within)
        );
        assert_eq!(
            wait(t, start, Some(d("2026-03-10")), d("2026-12-01")).1,
            TargetStatus::Past
        );
        // No target: never past, however long.
        let none = wait(None, start, None, d("2027-06-01"));
        assert_eq!(none.1, TargetStatus::NoTarget);
        assert!(none.0 > 500);
    }

    #[test]
    fn test_median_of_odd_even_and_empty() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3]), Some(3.0));
        assert_eq!(median(&[1, 2, 10]), Some(2.0));
        assert_eq!(median(&[1, 2, 4, 10]), Some(3.0));
    }
}
