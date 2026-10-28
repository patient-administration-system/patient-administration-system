//! Part of [`super`]: dims.

use super::*;

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
    pub(super) fn sql(self) -> &'static str {
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

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
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

pub(super) fn csv_field(v: &str) -> String {
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
    pub(super) fn sql(self) -> &'static str {
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

pub(super) fn rate(n: i64, of: i64) -> Option<f64> {
    (of > 0).then(|| ((n as f64 / of as f64) * 1000.0).round() / 1000.0)
}
