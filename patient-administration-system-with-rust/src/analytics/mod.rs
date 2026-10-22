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

use crate::db::row::RowExt;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;

use crate::config::env;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

mod budget;
mod demographics;
mod dims;
mod reports;
pub use dims::*;

pub struct AnalyticsService {
    db: DatabaseConnection,
}

impl AnalyticsService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
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
