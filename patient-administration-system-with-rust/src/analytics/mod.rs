//! A privacy-safe analytics feed (CAP-11).
//!
//! Counts of appointments by chosen dimensions (specialty, language, age band, deprivation quintile, time
//! of day, outcome, notice class), with no identifier anywhere in the output and small cells suppressed.
//!
//! - **Suppression.** A cell under `k` (default `PAS_ANALYTICS_K`, 10) is not shown: its count is withheld,
//!   not shown as zero. A caller can ask for a larger `k`, never a smaller one.
//! - **No differencing.** The grand total is published so the feed reconciles; so that a single hidden cell
//!   cannot be read off as `total - sum(visible)`, if exactly one cell is suppressed the smallest other cell
//!   is suppressed with it (secondary suppression).
//! - **Unknown is a value.** An appointment missing a dimension counts under `unknown`, so totals reconcile.
//!
//! Age band and deprivation quintile come from `person_demographics`, which an upstream job fills with the
//! coarse bands only. Time of day is in the clinic's time zone.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Specialty,
    Language,
    AgeBand,
    DeprivationQuintile,
    TimeOfDay,
    Outcome,
    Notice,
}

impl Dim {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "specialty" => Self::Specialty,
            "language" => Self::Language,
            "age_band" => Self::AgeBand,
            "deprivation_quintile" => Self::DeprivationQuintile,
            "time_of_day" => Self::TimeOfDay,
            "outcome" => Self::Outcome,
            "notice" => Self::Notice,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Specialty => "specialty",
            Self::Language => "language",
            Self::AgeBand => "age_band",
            Self::DeprivationQuintile => "deprivation_quintile",
            Self::TimeOfDay => "time_of_day",
            Self::Outcome => "outcome",
            Self::Notice => "notice",
        }
    }

    /// The SQL expression for this dimension. A fixed string per variant, never built from input.
    fn sql(self) -> &'static str {
        match self {
            Self::Specialty => "COALESCE(sc.service_type, 'unknown')",
            Self::Language => "COALESCE(cp.preferred_language, 'unknown')",
            Self::AgeBand => "COALESCE(d.age_band, 'unknown')",
            Self::DeprivationQuintile => "COALESCE(d.deprivation_quintile::text, 'unknown')",
            Self::TimeOfDay => {
                "CASE WHEN extract(hour FROM a.start_datetime AT TIME ZONE $3::text) < 12 THEN 'morning' \
                      WHEN extract(hour FROM a.start_datetime AT TIME ZONE $3::text) < 17 THEN 'afternoon' \
                      ELSE 'evening' END"
            }
            Self::Outcome => "a.status",
            Self::Notice => {
                "CASE WHEN a.status = 'cancelled' THEN CASE WHEN a.notice_days >= 3 THEN 'early_release' ELSE 'late_release' END \
                      WHEN a.status = 'no_show' THEN 'silent_miss' ELSE 'not_released' END"
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Cell {
    pub dims: Vec<(String, String)>,
    /// `None` when the cell is suppressed.
    pub count: Option<i64>,
    pub suppressed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub group_by: Vec<String>,
    pub k: i64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// Every appointment in the range, counted or not: what the cells reconcile to.
    pub total: i64,
    pub suppressed_cells: usize,
    pub cells: Vec<Cell>,
}

impl Report {
    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        let mut head: Vec<&str> = self.group_by.iter().map(String::as_str).collect();
        head.push("count");
        out.push_str(&head.join(","));
        out.push('\n');
        for c in &self.cells {
            let mut row: Vec<String> = c.dims.iter().map(|(_, v)| csv_field(v)).collect();
            row.push(c.count.map_or("suppressed".into(), |n| n.to_string()));
            out.push_str(&row.join(","));
            out.push('\n');
        }
        out
    }
}

fn csv_field(v: &str) -> String {
    // Also stops a value starting with = + - @ from running as a formula in a spreadsheet.
    let v = if v.starts_with(['=', '+', '-', '@']) {
        format!("'{v}")
    } else {
        v.to_string()
    };
    if v.contains([',', '"', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

/// Apply primary and secondary suppression to `(dims, true count)` rows. Pure, so it is easy to test.
pub fn suppress(rows: Vec<(Vec<(String, String)>, i64)>, k: i64) -> Vec<Cell> {
    let mut cells: Vec<Cell> = rows
        .into_iter()
        .map(|(dims, n)| {
            let hide = n < k;
            Cell {
                dims,
                count: (!hide).then_some(n),
                suppressed: hide,
            }
        })
        .collect();
    // One hidden cell would be total minus the rest: hide the smallest visible one with it.
    if cells.iter().filter(|c| c.suppressed).count() == 1
        && let Some(i) = cells
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.suppressed)
            .min_by_key(|(_, c)| c.count)
            .map(|(i, _)| i)
    {
        cells[i].count = None;
        cells[i].suppressed = true;
    }
    cells
}

/// Dimensions for the refill-rate report (PILOT-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefillDim {
    NoticeBand,
    ReleaseClass,
    Specialty,
    Language,
    AgeBand,
    DeprivationQuintile,
}

impl RefillDim {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "notice_band" => Self::NoticeBand,
            "release_class" => Self::ReleaseClass,
            "specialty" => Self::Specialty,
            "language" => Self::Language,
            "age_band" => Self::AgeBand,
            "deprivation_quintile" => Self::DeprivationQuintile,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::NoticeBand => "notice_band",
            Self::ReleaseClass => "release_class",
            Self::Specialty => "specialty",
            Self::Language => "language",
            Self::AgeBand => "age_band",
            Self::DeprivationQuintile => "deprivation_quintile",
        }
    }

    /// A fixed expression per variant, never built from input. `r` is the release, `cp` the communication
    /// profile and `d` the demographic bands of the patient who gave the slot up.
    fn sql(self) -> &'static str {
        match self {
            Self::NoticeBand => {
                "CASE WHEN r.notice_days IS NULL THEN 'silent_miss' WHEN r.notice_days = 0 THEN '0_days' \
                      WHEN r.notice_days < 3 THEN '1_2_days' WHEN r.notice_days < 7 THEN '3_6_days' ELSE '7_plus_days' END"
            }
            Self::ReleaseClass => "r.release_class",
            Self::Specialty => "COALESCE(r.service_type, 'unknown')",
            Self::Language => "COALESCE(cp.preferred_language, 'unknown')",
            Self::AgeBand => "COALESCE(d.age_band, 'unknown')",
            Self::DeprivationQuintile => "COALESCE(d.deprivation_quintile::text, 'unknown')",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RefillCell {
    pub dims: Vec<(String, String)>,
    /// Releases whose outcome is known: refilled, or the slot's time has passed unfilled.
    pub resolved: Option<i64>,
    pub refilled: Option<i64>,
    /// `refilled / resolved`, to three places.
    pub refill_rate: Option<f64>,
    /// The share of those releases that were early (three or more days' notice).
    pub early_release_share: Option<f64>,
    pub suppressed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefillReport {
    pub group_by: Vec<String>,
    pub k: i64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub total_resolved: i64,
    pub overall_refill_rate: Option<f64>,
    pub suppressed_cells: usize,
    pub cells: Vec<RefillCell>,
}

impl RefillReport {
    pub fn to_csv(&self) -> String {
        let mut head: Vec<&str> = self.group_by.iter().map(String::as_str).collect();
        head.extend(["resolved", "refilled", "refill_rate", "early_release_share"]);
        let mut out = head.join(",") + "\n";
        let num = |v: Option<f64>| v.map_or("suppressed".to_string(), |x| format!("{x:.3}"));
        let int = |v: Option<i64>| v.map_or("suppressed".to_string(), |x| x.to_string());
        for c in &self.cells {
            let mut row: Vec<String> = c.dims.iter().map(|(_, v)| csv_field(v)).collect();
            row.extend([
                int(c.resolved),
                int(c.refilled),
                num(c.refill_rate),
                num(c.early_release_share),
            ]);
            out.push_str(&(row.join(",") + "\n"));
        }
        out
    }
}

fn rate(n: i64, of: i64) -> Option<f64> {
    (of > 0).then(|| ((n as f64 / of as f64) * 1000.0).round() / 1000.0)
}

pub struct AnalyticsService {
    db: DatabaseConnection,
}

impl AnalyticsService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Per-caller query budget for a day (`PAS_ANALYTICS_QUERY_BUDGET`, default 100).
    ///
    /// Suppression protects one answer. Someone who may ask many differently shaped questions can still
    /// subtract one answer from another to recover a hidden cell (a window that differs by a week, a split that
    /// adds a dimension). The PAS cannot make that impossible for a free-form feed, so it makes it **bounded and
    /// visible**: every query is recorded in the audit log with its shape, and a caller who has used the
    /// day's budget is refused until tomorrow. The rest of the defence is people: who is given the
    /// feed, and the data-sharing terms they sign (`governance/analytics-differencing.md`).
    pub fn query_budget() -> i64 {
        std::env::var("PAS_ANALYTICS_QUERY_BUDGET")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(100)
            .clamp(1, 10_000)
    }

    /// Record one query against the caller's daily budget, or refuse it. Anonymous callers share one budget.
    pub async fn charge_query(
        &self,
        report: &str,
        shape: serde_json::Value,
        ctx: &UserContext,
    ) -> Result<()> {
        let who = ctx
            .user_id
            .clone()
            .unwrap_or_else(|| "anonymous".to_string());
        let used = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS n FROM audit_log WHERE action = 'analytics_query' \
                 AND COALESCE(user_id, 'anonymous') = $1 AND at > now() - interval '24 hours'",
                [who.clone().into()],
            ))
            .await
            .map_err(Error::Database)?
            .and_then(|r| r.try_get::<i64>("", "n").ok())
            .unwrap_or(0);
        if used >= Self::query_budget() {
            return Err(Error::forbidden(
                "the day's analytics query budget has been used; it renews 24 hours after each query",
            ));
        }
        AuditLogRepository::log(
            &self.db,
            "analytics",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, report.as_bytes()),
            "analytics_query",
            None,
            Some(shape),
            ctx,
        )
        .await
    }

    /// The smallest cell the feed will ever show: `PAS_ANALYTICS_K`, default 10, never below 5.
    pub fn minimum_k() -> i64 {
        std::env::var("PAS_ANALYTICS_K")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(10)
            .max(5)
    }

    /// Record an upstream person's coarse bands. Nothing else about them is taken.
    pub async fn set_demographics(
        &self,
        person_id: uuid::Uuid,
        age_band: Option<&str>,
        quintile: Option<i16>,
        source: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        if source.trim().is_empty() {
            return Err(Error::validation(
                "source must say where the bands come from",
            ));
        }
        if let Some(q) = quintile
            && !(1..=5).contains(&q)
        {
            return Err(Error::validation("deprivation_quintile is 1 to 5"));
        }
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO person_demographics (person_id, age_band, deprivation_quintile, source) VALUES ($1,$2,$3,$4) \
                 ON CONFLICT (person_id) DO UPDATE SET age_band = $2, deprivation_quintile = $3, source = $4, updated_at = now()",
                [
                    person_id.into(),
                    age_band.map(String::from).into(),
                    quintile.into(),
                    source.trim().into(),
                ],
            ))
            .await
            .map_err(|e| match e {
                sea_orm::DbErr::Exec(_) | sea_orm::DbErr::Query(_) => {
                    Error::validation("age_band must be one of 0-15, 16-24, 25-34, 35-44, 45-54, 55-64, 65-74, 75+")
                }
                other => Error::Database(other),
            })?;
        AuditLogRepository::log(
            &self.db,
            "patient",
            person_id,
            "demographic_bands_set",
            None,
            Some(serde_json::json!({ "source": source.trim() })),
            ctx,
        )
        .await
    }

    /// Load a postcode to quintile lookup the health board supplies, as CSV lines `postcode,quintile` (a header
    /// line is allowed). Replaces any earlier row for the same postcode. Bad lines are counted and skipped,
    /// never guessed at. The PAS ships no data of its own.
    pub async fn load_deprivation_lookup(
        &self,
        csv: &str,
        dataset: &str,
        loaded_by: &str,
        ctx: &UserContext,
    ) -> Result<(u64, u64)> {
        let (dataset, by) = (dataset.trim(), loaded_by.trim());
        if dataset.is_empty() || by.is_empty() {
            return Err(Error::validation(
                "name the dataset (for example WIMD 2019 with the postcode file used) and who loaded it",
            ));
        }
        let (mut loaded, mut rejected) = (0u64, 0u64);
        let rows: Vec<(String, i16)> = csv
            .lines()
            .filter_map(|l| {
                let mut it = l.split(',');
                let (pc, q) = (it.next()?, it.next()?);
                if it.next().is_some() {
                    return None;
                }
                let pc = normalise_postcode(pc);
                let q: i16 = q.trim().parse().ok()?;
                ((1..=5).contains(&q) && is_postcode(&pc)).then_some((pc, q))
            })
            .collect();
        let total = csv.lines().filter(|l| !l.trim().is_empty()).count() as u64;
        for chunk in rows.chunks(1000) {
            let mut sql = String::from(
                "INSERT INTO deprivation_lookup (postcode, quintile, dataset, loaded_by) VALUES ",
            );
            let mut values: Vec<sea_orm::Value> = Vec::new();
            for (i, (pc, q)) in chunk.iter().enumerate() {
                if i > 0 {
                    sql.push(',');
                }
                let b = values.len();
                sql.push_str(&format!("(${},${},${},${})", b + 1, b + 2, b + 3, b + 4));
                values.extend([pc.clone().into(), (*q).into(), dataset.into(), by.into()]);
            }
            sql.push_str(" ON CONFLICT (postcode) DO UPDATE SET quintile = EXCLUDED.quintile, dataset = EXCLUDED.dataset, loaded_by = EXCLUDED.loaded_by, loaded_at = now()");
            self.db
                .execute(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    sql,
                    values,
                ))
                .await
                .map_err(Error::Database)?;
            loaded += chunk.len() as u64;
        }
        // A header line is not a rejection; anything else that did not parse is.
        let header = csv
            .lines()
            .next()
            .is_some_and(|l| l.to_lowercase().contains("postcode"));
        rejected += total.saturating_sub(loaded + u64::from(header));
        AuditLogRepository::log(
            &self.db,
            "deprivation_lookup",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, dataset.as_bytes()),
            "deprivation_lookup_loaded",
            None,
            Some(serde_json::json!({ "dataset": dataset, "loaded_by": by, "loaded": loaded, "rejected": rejected })),
            ctx,
        )
        .await?;
        Ok((loaded, rejected))
    }

    /// Fill `person_demographics` for people who have appointments and no bands yet (or stale ones): the age
    /// band from the date of birth and the quintile from the postcode, looked up in the board's own table.
    /// Only the two coarse bands are kept; the date of birth and postcode are read, used and not stored.
    /// A person with no usable date or no matching postcode gets the part that is known. Returns how many
    /// people were updated.
    pub async fn refresh_demographics(
        &self,
        people: &dyn crate::upstream::PersonClient,
        limit: i64,
        upstream_ctx: &crate::upstream::UserContext,
        ctx: &UserContext,
    ) -> Result<u64> {
        let ids = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT a.person_id FROM appointments a \
                 LEFT JOIN person_demographics d ON d.person_id = a.person_id \
                 WHERE a.deleted_at IS NULL AND (d.person_id IS NULL OR d.updated_at < now() - interval '90 days') \
                 GROUP BY a.person_id ORDER BY max(a.updated_at) DESC LIMIT $1",
                [limit.clamp(1, 5000).into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut n = 0;
        for r in ids {
            let id: uuid::Uuid = r.try_get("", "person_id").map_err(Error::Database)?;
            // A person the upstream cannot return is skipped, not an error for the whole run.
            let Ok(person) = people.get(id, upstream_ctx).await else {
                continue;
            };
            let band = person
                .data
                .get("birth_date")
                .or_else(|| person.data.get("birthDate"))
                .and_then(|v| v.as_str())
                .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .map(|d| age_band(d, Utc::now().date_naive()));
            let postcode = person
                .data
                .get("addresses")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.get("postal_code").or_else(|| a.get("postalCode")))
                .and_then(|v| v.as_str())
                .map(normalise_postcode);
            let (quintile, dataset): (Option<i16>, Option<String>) = match postcode {
                Some(pc) => self
                    .db
                    .query_one(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "SELECT quintile, dataset FROM deprivation_lookup WHERE postcode = $1",
                        [pc.into()],
                    ))
                    .await
                    .map_err(Error::Database)?
                    .map(|r| {
                        (
                            r.try_get("", "quintile").ok(),
                            r.try_get("", "dataset").ok(),
                        )
                    })
                    .unwrap_or((None, None)),
                None => (None, None),
            };
            if band.is_none() && quintile.is_none() {
                continue;
            }
            let source = format!("refresh:{}", dataset.as_deref().unwrap_or("age-only"));
            self.set_demographics(id, band, quintile, &source, ctx)
                .await?;
            n += 1;
        }
        Ok(n)
    }

    /// Refill rate of released slots, split by the chosen dimensions (PILOT-10). Releases are counted by the
    /// day they were released; only those with a known outcome count (refilled, or the slot's time passed
    /// unfilled), so a slot still open for next week does not look like a failure. A cell under `k`
    /// resolved releases is withheld whole, and the same secondary suppression as [`Self::outcomes`] applies.
    pub async fn refill_rates(
        &self,
        group_by: &[RefillDim],
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        k: Option<i64>,
    ) -> Result<RefillReport> {
        if group_by.is_empty() || group_by.len() > 4 {
            return Err(Error::validation("group_by takes one to four dimensions"));
        }
        let mut seen = Vec::new();
        for d in group_by {
            if seen.contains(d) {
                return Err(Error::validation("a dimension may be named once"));
            }
            seen.push(*d);
        }
        let k = k.unwrap_or(0).max(Self::minimum_k());
        // Release times are stamped by the database's clock, which can be a little ahead of this server's, so
        // the default end is a few minutes ahead of now rather than exactly now.
        let to = to.unwrap_or_else(|| Utc::now() + chrono::Duration::minutes(5));
        let from = from.unwrap_or_else(|| to - chrono::Duration::days(90));
        let select: Vec<String> = group_by
            .iter()
            .enumerate()
            .map(|(i, d)| format!("{} AS d{i}", d.sql()))
            .collect();
        let group: Vec<String> = (1..=group_by.len()).map(|i| i.to_string()).collect();
        let sql = format!(
            "SELECT {sel}, count(*)::bigint AS n, \
                    count(*) FILTER (WHERE r.status = 'refilled')::bigint AS refilled, \
                    count(*) FILTER (WHERE r.release_class = 'early_release')::bigint AS early \
             FROM slot_releases r \
             LEFT JOIN appointments a ON a.id = r.appointment_id \
             LEFT JOIN patient_communication_profiles cp ON cp.person_id = a.person_id \
             LEFT JOIN person_demographics d ON d.person_id = a.person_id \
             WHERE r.released_at >= $1 AND r.released_at < $2 \
               AND (r.status = 'refilled' OR r.start_datetime <= now()) \
             GROUP BY {grp} ORDER BY {grp}",
            sel = select.join(", "),
            grp = group.join(", "),
        );
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                [from.into(), to.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut data = Vec::new();
        let mut extra = Vec::new();
        let (mut total, mut total_refilled) = (0, 0);
        for r in &rows {
            let dims = group_by
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    Ok((
                        d.name().to_string(),
                        r.try_get::<String>("", &format!("d{i}"))
                            .map_err(Error::Database)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let n: i64 = r.try_get("", "n").map_err(Error::Database)?;
            let refilled: i64 = r.try_get("", "refilled").map_err(Error::Database)?;
            let early: i64 = r.try_get("", "early").map_err(Error::Database)?;
            total += n;
            total_refilled += refilled;
            data.push((dims, n));
            extra.push((refilled, early));
        }
        let flags = suppress(data, k);
        let cells: Vec<RefillCell> = flags
            .into_iter()
            .zip(extra)
            .map(|(c, (refilled, early))| match c.count {
                Some(n) => RefillCell {
                    dims: c.dims,
                    resolved: Some(n),
                    refilled: Some(refilled),
                    refill_rate: rate(refilled, n),
                    early_release_share: rate(early, n),
                    suppressed: false,
                },
                None => RefillCell {
                    dims: c.dims,
                    resolved: None,
                    refilled: None,
                    refill_rate: None,
                    early_release_share: None,
                    suppressed: true,
                },
            })
            .collect();
        Ok(RefillReport {
            group_by: group_by.iter().map(|d| d.name().to_string()).collect(),
            k,
            from,
            to,
            total_resolved: total,
            overall_refill_rate: rate(total_refilled, total),
            suppressed_cells: cells.iter().filter(|c| c.suppressed).count(),
            cells,
        })
    }

    pub async fn outcomes(
        &self,
        group_by: &[Dim],
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        k: Option<i64>,
    ) -> Result<Report> {
        if group_by.is_empty() || group_by.len() > 4 {
            return Err(Error::validation("group_by takes one to four dimensions"));
        }
        let mut seen = Vec::new();
        for d in group_by {
            if seen.contains(d) {
                return Err(Error::validation("a dimension may be named once"));
            }
            seen.push(*d);
        }
        let k = k.unwrap_or(0).max(Self::minimum_k());
        let to = to.unwrap_or_else(|| Utc::now() + chrono::Duration::minutes(5));
        let from = from.unwrap_or_else(|| to - chrono::Duration::days(90));
        let tz = crate::inbound::messages::timezone().name().to_string();

        let select: Vec<String> = group_by
            .iter()
            .enumerate()
            .map(|(i, d)| format!("{} AS d{i}", d.sql()))
            .collect();
        let group: Vec<String> = (1..=group_by.len()).map(|i| i.to_string()).collect();
        let sql = format!(
            "SELECT {sel}, count(*)::bigint AS n \
             FROM appointments a \
             LEFT JOIN slots sl ON sl.id = a.slot_id \
             LEFT JOIN schedules sc ON sc.id = sl.schedule_id \
             LEFT JOIN patient_communication_profiles cp ON cp.person_id = a.person_id \
             LEFT JOIN person_demographics d ON d.person_id = a.person_id \
             WHERE a.deleted_at IS NULL AND a.start_datetime >= $1 AND a.start_datetime < $2 \
             GROUP BY {grp} ORDER BY {grp}",
            sel = select.join(", "),
            grp = group.join(", "),
        );
        // The time zone is a parameter only when a dimension uses it.
        let mut values: Vec<sea_orm::Value> = vec![from.into(), to.into()];
        if group_by.contains(&Dim::TimeOfDay) {
            values.push(tz.into());
        }
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                values,
            ))
            .await
            .map_err(Error::Database)?;
        let mut data = Vec::new();
        let mut total = 0;
        for r in &rows {
            let dims = group_by
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    Ok((
                        d.name().to_string(),
                        r.try_get::<String>("", &format!("d{i}"))
                            .map_err(Error::Database)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let n: i64 = r.try_get("", "n").map_err(Error::Database)?;
            total += n;
            data.push((dims, n));
        }
        let cells = suppress(data, k);
        Ok(Report {
            group_by: group_by.iter().map(|d| d.name().to_string()).collect(),
            k,
            from,
            to,
            total,
            suppressed_cells: cells.iter().filter(|c| c.suppressed).count(),
            cells,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(counts: &[i64]) -> Vec<(Vec<(String, String)>, i64)> {
        counts
            .iter()
            .enumerate()
            .map(|(i, n)| (vec![("d".to_string(), format!("v{i}"))], *n))
            .collect()
    }

    #[test]
    fn test_a_cell_under_k_is_hidden_not_zero() {
        let c = suppress(rows(&[50, 3, 40, 2, 30]), 10);
        assert_eq!(c[1].count, None);
        assert!(c[1].suppressed);
        assert_eq!(c[3].count, None);
        assert_eq!(c[0].count, Some(50));
    }

    #[test]
    fn test_a_single_hidden_cell_takes_the_smallest_visible_one_with_it() {
        let c = suppress(rows(&[50, 3, 40, 12]), 10);
        assert!(c[1].suppressed, "the small one");
        assert!(
            c[3].suppressed,
            "and the smallest visible, so the pair cannot be separated"
        );
        assert_eq!(c[0].count, Some(50));
        assert_eq!(c[2].count, Some(40));
    }

    #[test]
    fn test_nothing_hidden_when_nothing_is_small() {
        let c = suppress(rows(&[50, 40, 12]), 10);
        assert!(c.iter().all(|c| !c.suppressed));
    }

    #[test]
    fn test_csv_marks_suppressed_cells_and_defuses_formulas() {
        let r = Report {
            group_by: vec!["specialty".into()],
            k: 10,
            from: Utc::now(),
            to: Utc::now(),
            total: 0,
            suppressed_cells: 1,
            cells: vec![
                Cell {
                    dims: vec![("specialty".into(), "=cmd".into())],
                    count: Some(12),
                    suppressed: false,
                },
                Cell {
                    dims: vec![("specialty".into(), "a,b".into())],
                    count: None,
                    suppressed: true,
                },
            ],
        };
        let csv = r.to_csv();
        assert!(csv.contains("'=cmd,12"), "{csv}");
        assert!(csv.contains("\"a,b\",suppressed"), "{csv}");
    }
}

/// Uppercase, no spaces: how postcodes are held in the lookup.
pub fn normalise_postcode(p: &str) -> String {
    p.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

/// A plausible UK postcode shape (5 to 7 letters and digits, a digit before the last three characters); the
/// lookup is the authority on whether it exists.
fn is_postcode(p: &str) -> bool {
    (5..=7).contains(&p.len())
        && p.chars().all(|c| c.is_ascii_alphanumeric())
        && p.chars().rev().nth(2).is_some_and(|c| c.is_ascii_digit())
}

/// The coarse band for a date of birth on a given day (`0-15` ... `75+`).
pub fn age_band(born: chrono::NaiveDate, today: chrono::NaiveDate) -> &'static str {
    let mut y = today.years_since(born).unwrap_or(0);
    if born > today {
        y = 0;
    }
    match y {
        0..=15 => "0-15",
        16..=24 => "16-24",
        25..=34 => "25-34",
        35..=44 => "35-44",
        45..=54 => "45-54",
        55..=64 => "55-64",
        65..=74 => "65-74",
        _ => "75+",
    }
}

#[cfg(test)]
mod feed_tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_age_bands_follow_the_birthday_not_the_year() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        let today = d(2026, 10, 19);
        assert_eq!(age_band(d(2010, 10, 19), today), "16-24", "16 today");
        assert_eq!(
            age_band(d(2010, 10, 20), today),
            "0-15",
            "15 until tomorrow"
        );
        assert_eq!(age_band(d(1951, 10, 20), today), "65-74");
        assert_eq!(age_band(d(1951, 10, 19), today), "75+");
        assert_eq!(
            age_band(d(2030, 1, 1), today),
            "0-15",
            "a future date is not a crash"
        );
    }

    #[test]
    fn test_postcodes_are_normalised_and_checked_for_shape() {
        assert_eq!(normalise_postcode(" cf10 1ab "), "CF101AB");
        assert!(is_postcode("CF101AB") && is_postcode("M11AE") && is_postcode("SA612AB"));
        assert!(!is_postcode("") && !is_postcode("HELLO") && !is_postcode("CF10-1AB"));
    }
}
