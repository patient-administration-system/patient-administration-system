//! Suspected conditions: tracking suspicion during referrals (WPAS-1, spec 20, FR-207 to FR-216).
//!
//! A referral is often made because a clinician *suspects* a condition: cancer, sleep apnoea, diabetes,
//! cardiovascular disease, Parkinson's, Alzheimer's. The wait that matters starts at that point of suspicion.
//! This module records the suspicion and keeps its wait visible until its clock stops.
//!
//! - **The PAS decides nothing.** Staff record what a named clinician suspected and decided (`suspected_by`,
//!   `stopped_by`); the module never suspects, downgrades or ranks.
//! - **The clock never pauses** (FR-208). Days waited are whole days from the point of suspicion to the stop
//!   date, or to today in the service's time zone (`PAS_TIMEZONE`) while open.
//! - **It stops once** (FR-209), for `treatment_started`, `downgraded`, `died` or `entered_in_error`.
//! - **No invented targets** (FR-211): cancer has the Welsh Suspected Cancer Pathway's 62 days; every other
//!   condition has none until a lead names one.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Row, Stmt, Transact};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::{Error, Result};

crate::string_enum! {
    /// The condition a referral is made on suspicion of (spec 20). Closed and coded: adding one is a migration.
    pub enum SuspectedCondition {
        Cancer = "cancer",
        ObstructiveSleepApnoea = "obstructive_sleep_apnoea",
        Diabetes = "diabetes",
        CardiovascularDisease = "cardiovascular_disease",
        ParkinsonsDisease = "parkinsons_disease",
        AlzheimersDisease = "alzheimers_disease",
    }
}

crate::string_enum! {
    /// The primary cancer site, from the Welsh "Primary Cancer Site Description" list (NHS Wales Data
    /// Dictionary, valid from 2021-01-01, last changed by DSCN 2023/45).
    pub enum TumourSite {
        HeadAndNeck = "01",
        UpperGi = "02",
        LowerGi = "03",
        Lung = "04",
        Sarcoma = "05",
        /// Excluding basal cell carcinoma.
        Skin = "06",
        BrainCns = "07",
        Breast = "08",
        Gynaecological = "09",
        Urological = "10",
        /// Excluding acute leukaemia.
        Haematological = "11",
        AcuteLeukaemia = "12",
        Childrens = "13",
        UnknownPrimary = "14",
        Other = "98",
    }
}

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
        /// The condition has no published target (FR-211).
        NoTarget = "no_target",
        Within = "within",
        /// Open, and within [`NEAR_TARGET_DAYS`] of the target: a display aid, not a rule.
        Near = "near",
        Past = "past",
    }
}

/// How close to a target an open wait is marked `near` (FR-212).
pub const NEAR_TARGET_DAYS: i64 = 7;

/// The published target for a condition, in days, or none (FR-211).
pub fn target_days(condition: SuspectedCondition) -> Option<i64> {
    match condition {
        // Welsh Suspected Cancer Pathway: first definitive treatment within 62 days of the point of suspicion.
        SuspectedCondition::Cancer => Some(62),
        _ => None,
    }
}

/// Today in the service's time zone, so "days waited" turns over at local midnight.
pub fn today() -> NaiveDate {
    Utc::now()
        .with_timezone(&crate::inbound::messages::timezone())
        .date_naive()
}

/// Whole days waited and where they stand against the target (FR-208, FR-211, FR-212).
pub fn wait(
    condition: SuspectedCondition,
    suspected_on: NaiveDate,
    stopped_on: Option<NaiveDate>,
    today: NaiveDate,
) -> (i64, Option<i64>, TargetStatus) {
    let days = (stopped_on.unwrap_or(today) - suspected_on)
        .num_days()
        .max(0);
    let target = target_days(condition);
    let status = match target {
        None => TargetStatus::NoTarget,
        Some(t) if days > t => TargetStatus::Past,
        Some(t) if stopped_on.is_none() && days > t - NEAR_TARGET_DAYS => TargetStatus::Near,
        Some(_) => TargetStatus::Within,
    };
    (days, target, status)
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Suspicion {
    pub id: Uuid,
    pub referral_id: Uuid,
    pub person_id: Uuid,
    pub condition: SuspectedCondition,
    /// Cancer only.
    #[schema(required = true)]
    pub tumour_site: Option<TumourSite>,
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
    /// The published target in days, or none (FR-211).
    #[schema(required = true)]
    pub target_days: Option<i64>,
    pub target_status: TargetStatus,
    pub updated_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, referral_id, person_id, condition, tumour_site, unique_pathway_identifier, suspected_on, \
     suspected_by, recorded_by, recorded_at, stop_reason, stopped_on, stopped_by, updated_at";

fn from_row(r: &Row, today: NaiveDate) -> Result<Suspicion> {
    let condition: SuspectedCondition = r.col::<String>("condition")?.parse()?;
    let suspected_on: NaiveDate = r.col("suspected_on")?;
    let stopped_on: Option<NaiveDate> = r.col("stopped_on")?;
    let (days_waited, target_days, target_status) =
        wait(condition, suspected_on, stopped_on, today);
    Ok(Suspicion {
        id: r.col("id")?,
        referral_id: r.col("referral_id")?,
        person_id: r.col("person_id")?,
        condition,
        tumour_site: r
            .col::<Option<String>>("tumour_site")?
            .map(|s| s.parse())
            .transpose()?,
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
        updated_at: r.col("updated_at")?,
    })
}

/// What a person records: the condition, the point of suspicion and who suspected it.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct NewSuspicion {
    #[schema(value_type = SuspectedCondition)]
    pub condition: String,
    /// Required for cancer; refused for any other condition.
    #[serde(default)]
    #[schema(value_type = Option<TumourSite>)]
    pub tumour_site: Option<String>,
    #[serde(default)]
    pub unique_pathway_identifier: Option<String>,
    pub suspected_on: NaiveDate,
    pub suspected_by: String,
    pub recorded_by: String,
}

/// One row of the report (FR-213): suspicions whose clock stopped in the period.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReportRow {
    pub condition: SuspectedCondition,
    /// Cancer rows are per tumour site; other conditions have none.
    #[schema(required = true)]
    pub tumour_site: Option<TumourSite>,
    pub stopped: i64,
    /// None when the condition has no target.
    #[schema(required = true)]
    pub within_target: Option<i64>,
    #[schema(required = true)]
    pub share_within_target: Option<f64>,
    #[schema(required = true)]
    pub median_days: Option<f64>,
    #[schema(required = true)]
    pub target_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SuspicionReport {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub rows: Vec<ReportRow>,
}

/// Is there an open cancer suspicion on the referral this appointment was booked from? (FR-214)
pub async fn appointment_has_open_cancer_suspicion<C: Conn>(
    conn: &C,
    appointment_id: Uuid,
) -> Result<bool> {
    Ok(conn
        .one(Stmt::new(
            "SELECT 1 AS hit FROM appointments a JOIN suspected_conditions s ON s.referral_id = a.referral_id \
             WHERE a.id = $1 AND s.condition = 'cancer' AND s.stop_reason IS NULL LIMIT 1",
            [appointment_id.into()],
        ))
        .await?
        .is_some())
}

pub struct SuspicionService {
    db: Pool,
}

impl SuspicionService {
    pub fn new(db: Pool) -> Self {
        Self { db }
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
        from_row(&r, today())
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
        rows.iter().map(|r| from_row(r, today)).collect()
    }

    /// The open list (FR-212): every open suspicion, longest wait first, optionally of one condition or site.
    pub async fn open(
        &self,
        condition: Option<SuspectedCondition>,
        tumour_site: Option<TumourSite>,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<Suspicion>> {
        let rows = rows.into();
        self.list(
            "WHERE stop_reason IS NULL AND ($1::text IS NULL OR condition = $1) \
             AND ($2::text IS NULL OR tumour_site = $2) ORDER BY suspected_on, id LIMIT $3 OFFSET $4",
            vec![
                condition.map(|c| c.as_str().to_string()).into(),
                tumour_site.map(|s| s.as_str().to_string()).into(),
                rows.fetch().into(),
                rows.offset().into(),
            ],
        )
        .await
    }

    /// Record a suspicion on a referral (FR-207, FR-210, FR-215).
    pub async fn record(
        &self,
        referral_id: Uuid,
        n: NewSuspicion,
        ctx: &UserContext,
    ) -> Result<Suspicion> {
        let condition: SuspectedCondition = n.condition.parse().map_err(|_| {
            Error::validation(format!(
                "condition must be one of {}",
                SuspectedCondition::NAMES.join(", ")
            ))
        })?;
        let tumour_site = match (condition, n.tumour_site.as_deref()) {
            (SuspectedCondition::Cancer, None) => {
                return Err(Error::validation(
                    "a cancer suspicion needs a tumour_site from the primary cancer site list",
                ));
            }
            (SuspectedCondition::Cancer, Some(s)) => {
                Some(s.parse::<TumourSite>().map_err(|_| {
                    Error::validation(format!(
                        "tumour_site must be one of {}",
                        TumourSite::NAMES.join(", ")
                    ))
                })?)
            }
            (_, Some(_)) => {
                return Err(Error::validation(
                    "only a cancer suspicion has a tumour_site",
                ));
            }
            (_, None) => None,
        };
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
                            [referral_id.into(), condition.as_str().into()],
                        ))
                        .await?;
                    if open.is_some() {
                        return Err(Error::conflict(format!(
                            "that referral already has an open suspicion of {condition}"
                        )));
                    }
                    txn.run(Stmt::new(
                        "INSERT INTO suspected_conditions (id, referral_id, person_id, condition, tumour_site, \
                           unique_pathway_identifier, suspected_on, suspected_by, recorded_by) \
                         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                        [
                            id.into(),
                            referral_id.into(),
                            person_id.into(),
                            condition.as_str().into(),
                            tumour_site.map(|s| s.as_str().to_string()).into(),
                            upi.clone().into(),
                            n.suspected_on.into(),
                            suspected_by.clone().into(),
                            recorded_by.clone().into(),
                        ],
                    ))
                    .await?;
                    let detail = serde_json::json!({
                        "suspicion_id": id,
                        "referral_id": referral_id,
                        "person_id": person_id,
                        "condition": condition.as_str(),
                        "tumour_site": tumour_site.map(TumourSite::as_str),
                        "suspected_on": n.suspected_on,
                        "suspected_by": suspected_by,
                        "recorded_by": recorded_by,
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
                    Ok(())
                })
            })
            .await?;
        self.get(id).await
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
    /// condition and (for cancer) tumour site. `entered_in_error` is left out.
    pub async fn report(&self, from: NaiveDate, to: NaiveDate) -> Result<SuspicionReport> {
        if to < from {
            return Err(Error::validation("`from` must not be after `to`"));
        }
        let rows = self
            .db
            .all(Stmt::new(
                "SELECT condition, tumour_site, suspected_on, stopped_on FROM suspected_conditions \
                 WHERE stop_reason IS NOT NULL AND stop_reason <> 'entered_in_error' \
                   AND stopped_on BETWEEN $1 AND $2 \
                 ORDER BY condition, tumour_site",
                [from.into(), to.into()],
            ))
            .await?;
        let mut groups: std::collections::BTreeMap<(String, Option<String>), Vec<i64>> =
            std::collections::BTreeMap::new();
        for r in &rows {
            let start: NaiveDate = r.col("suspected_on")?;
            let stop: NaiveDate = r.col("stopped_on")?;
            groups
                .entry((r.col("condition")?, r.col("tumour_site")?))
                .or_default()
                .push((stop - start).num_days());
        }
        let mut out = Vec::new();
        for ((condition, site), mut days) in groups {
            let condition: SuspectedCondition = condition.parse()?;
            let target = target_days(condition);
            days.sort_unstable();
            let n = days.len() as i64;
            let within = target.map(|t| days.iter().filter(|d| **d <= t).count() as i64);
            out.push(ReportRow {
                condition,
                tumour_site: site.map(|s| s.parse()).transpose()?,
                stopped: n,
                within_target: within,
                share_within_target: within.map(|w| w as f64 / n as f64),
                median_days: median(&days),
                target_days: target,
            });
        }
        Ok(SuspicionReport {
            from,
            to,
            rows: out,
        })
    }
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
    fn test_cancer_has_62_days_and_nothing_else_has_a_target() {
        assert_eq!(target_days(SuspectedCondition::Cancer), Some(62));
        for c in SuspectedCondition::ALL {
            if *c != SuspectedCondition::Cancer {
                assert_eq!(
                    target_days(*c),
                    None,
                    "{c} must not have an invented target"
                );
            }
        }
    }

    #[test]
    fn test_the_wait_is_real_days_and_marks_near_and_past() {
        let start = d("2026-01-01");
        let c = SuspectedCondition::Cancer;
        assert_eq!(wait(c, start, None, d("2026-01-01")).0, 0);
        assert_eq!(
            wait(c, start, None, d("2026-02-25")).2,
            TargetStatus::Within,
            "day 55"
        );
        assert_eq!(
            wait(c, start, None, d("2026-02-26")).2,
            TargetStatus::Near,
            "day 56"
        );
        assert_eq!(
            wait(c, start, None, d("2026-03-04")).2,
            TargetStatus::Near,
            "day 62 is within, but near"
        );
        assert_eq!(
            wait(c, start, None, d("2026-03-05")).2,
            TargetStatus::Past,
            "day 63"
        );
        // A stopped clock is judged on its stop date, and is never "near".
        let stopped = wait(c, start, Some(d("2026-03-01")), d("2026-12-01"));
        assert_eq!((stopped.0, stopped.2), (59, TargetStatus::Within));
        let late = wait(c, start, Some(d("2026-03-10")), d("2026-12-01"));
        assert_eq!(late.2, TargetStatus::Past);
        // No target: never past, however long.
        let osa = wait(
            SuspectedCondition::ObstructiveSleepApnoea,
            start,
            None,
            d("2027-06-01"),
        );
        assert_eq!(osa.2, TargetStatus::NoTarget);
        assert!(osa.0 > 500);
    }

    #[test]
    fn test_median_of_odd_even_and_empty() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3]), Some(3.0));
        assert_eq!(median(&[1, 2, 10]), Some(2.0));
        assert_eq!(median(&[1, 2, 4, 10]), Some(3.0));
    }

    #[test]
    fn test_the_tumour_site_codes_are_the_welsh_list() {
        assert_eq!(
            TumourSite::NAMES,
            [
                "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14",
                "98"
            ]
        );
    }
}
