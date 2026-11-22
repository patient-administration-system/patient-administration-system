//! Working days (TSC-3): which dates a team works, and arithmetic in working days.
//!
//! "Due in two working days" and "remind three working days before" skip the days nobody is in. Which days
//! those are is configuration, never code: the days of the week off (`PAS_NON_WORKING_DAYS`, default
//! `sat,sun`) and the public holidays as dates (`PAS_HOLIDAYS`, none by default). Holidays differ by nation
//! and region and are announced year by year, so the PAS names none of them (NFR-26, QUAL-40); a deployer
//! lists theirs.
//!
//! The arithmetic is on dates in the service's own calendar, not instants: a caller turns an instant into a
//! local date first (the service time zone, `PAS_TIMEZONE`).

use std::collections::BTreeSet;

use jiff::civil::{Date, Weekday};

use crate::config::env;
use crate::{Error, Result};

/// The setting for the days of the week off.
pub const NON_WORKING_DAYS: &str = "PAS_NON_WORKING_DAYS";
/// The setting for the public holidays, as ISO dates.
pub const HOLIDAYS: &str = "PAS_HOLIDAYS";

/// A year of steps is the most any one call walks; a request beyond it is refused rather than looped over.
const MAX_STEPS: u32 = 366 * 5;

/// The days a team works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingCalendar {
    /// Indexed by `Weekday::num_days_from_monday`.
    days_off: [bool; 7],
    holidays: BTreeSet<Date>,
}

impl Default for WorkingCalendar {
    /// Saturday and Sunday off, and no holidays.
    fn default() -> Self {
        Self {
            days_off: off([Weekday::Saturday, Weekday::Sunday]),
            holidays: BTreeSet::new(),
        }
    }
}

impl WorkingCalendar {
    /// A calendar with these days of the week off and these holidays. At least one day of the week must be
    /// a working day.
    pub fn new(
        days_off: impl IntoIterator<Item = Weekday>,
        holidays: impl IntoIterator<Item = Date>,
    ) -> Result<Self> {
        let days_off = off(days_off);
        if days_off.iter().all(|d| *d) {
            return Err(Error::config(format!(
                "{NON_WORKING_DAYS}: at least one day of the week must be a working day"
            )));
        }
        Ok(Self {
            days_off,
            holidays: holidays.into_iter().collect(),
        })
    }

    /// Read from the two settings; unset means the default (Saturday and Sunday off, no holidays).
    pub fn from_env() -> Result<Self> {
        Self::parse(
            env::text(NON_WORKING_DAYS).as_deref(),
            env::text(HOLIDAYS).as_deref(),
        )
    }

    /// Parse the two settings' text: days as three-letter names (`sat,sun`; `none` for a team that works
    /// every day), holidays as ISO dates (`2026-12-25,2026-12-28`). Blank means the default.
    pub fn parse(days_off: Option<&str>, holidays: Option<&str>) -> Result<Self> {
        let days = match days_off.map(str::trim).filter(|s| !s.is_empty()) {
            None => Self::default().days_off,
            Some(s) if s.eq_ignore_ascii_case("none") => [false; 7],
            Some(s) => s
                .split(',')
                .map(|d| parse_weekday(d.trim()))
                .collect::<Result<Vec<_>>>()
                .map(off)?,
        };
        let dates = match holidays.map(str::trim).filter(|s| !s.is_empty()) {
            None => BTreeSet::new(),
            Some(s) => s
                .split(',')
                .map(|d| {
                    d.trim().parse::<Date>().map_err(|_| {
                        Error::config(format!(
                            "{HOLIDAYS}: {:?} is not a date such as 2026-12-25",
                            d.trim()
                        ))
                    })
                })
                .collect::<Result<BTreeSet<_>>>()?,
        };
        if days.iter().all(|d| *d) {
            return Err(Error::config(format!(
                "{NON_WORKING_DAYS}: at least one day of the week must be a working day"
            )));
        }
        Ok(Self {
            days_off: days,
            holidays: dates,
        })
    }

    /// Whether `date` is a working day.
    pub fn is_working_day(&self, date: Date) -> bool {
        !self
            .days_off
            .get(crate::time::monday_index(date.weekday()))
            .copied()
            .unwrap_or(false)
            && !self.holidays.contains(&date)
    }

    /// The working day `n` working days after `date`. With `n = 0`, `date` itself if it is a working day,
    /// otherwise the next one: "due today" on a Saturday is due on Monday.
    pub fn add_working_days(&self, date: Date, n: u32) -> Result<Date> {
        self.walk(date, n, true)
    }

    /// The working day `n` working days before `date`. With `n = 0`, `date` itself if it is a working day,
    /// otherwise the working day before it: "send by" a Sunday is sent on the Friday.
    pub fn sub_working_days(&self, date: Date, n: u32) -> Result<Date> {
        self.walk(date, n, false)
    }

    /// How many working days lie after `from` up to and including `to` (zero when `to` is not after `from`).
    pub fn working_days_between(&self, from: Date, to: Date) -> u32 {
        let mut count = 0;
        let mut day = from;
        while day < to {
            match day.tomorrow() {
                Ok(next) => day = next,
                Err(_) => break,
            }
            if self.is_working_day(day) {
                count += 1;
            }
        }
        count
    }

    fn walk(&self, date: Date, n: u32, forward: bool) -> Result<Date> {
        let step = |d: Date| {
            if forward {
                d.tomorrow().ok()
            } else {
                d.yesterday().ok()
            }
        };
        let out_of_range = || Error::validation("the date is out of range");
        let mut day = date;
        let mut steps = 0u32;
        while !self.is_working_day(day) {
            day = step(day).ok_or_else(out_of_range)?;
            steps += 1;
            if steps > MAX_STEPS {
                return Err(out_of_range());
            }
        }
        let mut left = n;
        while left > 0 {
            day = step(day).ok_or_else(out_of_range)?;
            steps += 1;
            if steps > MAX_STEPS {
                return Err(out_of_range());
            }
            if self.is_working_day(day) {
                left -= 1;
            }
        }
        Ok(day)
    }
}

/// The seven-day mask with `days` off.
fn off(days: impl IntoIterator<Item = Weekday>) -> [bool; 7] {
    let mut mask = [false; 7];
    for d in days {
        if let Some(slot) = mask.get_mut(crate::time::monday_index(d)) {
            *slot = true;
        }
    }
    mask
}

fn parse_weekday(s: &str) -> Result<Weekday> {
    let lower = s.to_ascii_lowercase();
    let day = match lower.get(..3) {
        Some("mon") => Weekday::Monday,
        Some("tue") => Weekday::Tuesday,
        Some("wed") => Weekday::Wednesday,
        Some("thu") => Weekday::Thursday,
        Some("fri") => Weekday::Friday,
        Some("sat") => Weekday::Saturday,
        Some("sun") => Weekday::Sunday,
        _ => {
            return Err(Error::config(format!(
                "{NON_WORKING_DAYS}: {s:?} is not a day such as sat or sun"
            )));
        }
    };
    Ok(day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i16, m: i8, day: i8) -> Date {
        jiff::civil::date(y, m, day)
    }

    /// 2026-12-24 is a Thursday; 25th Friday, 26th Saturday, 27th Sunday, 28th Monday.
    fn christmas() -> WorkingCalendar {
        WorkingCalendar::parse(None, Some("2026-12-25, 2026-12-28")).unwrap()
    }

    #[test]
    fn a_weekend_is_skipped_forwards_and_backwards() {
        let c = WorkingCalendar::default();
        // Friday 2026-11-20 plus one working day is Monday the 23rd.
        assert_eq!(
            c.add_working_days(d(2026, 11, 20), 1).unwrap(),
            d(2026, 11, 23)
        );
        // Monday minus one is the Friday before.
        assert_eq!(
            c.sub_working_days(d(2026, 11, 23), 1).unwrap(),
            d(2026, 11, 20)
        );
        // Five working days from a Monday is the next Monday.
        assert_eq!(
            c.add_working_days(d(2026, 11, 16), 5).unwrap(),
            d(2026, 11, 23)
        );
    }

    #[test]
    fn zero_days_lands_on_a_working_day() {
        let c = WorkingCalendar::default();
        // Due "today" on a Saturday is due on the Monday; sent "by" a Sunday is sent on the Friday.
        assert_eq!(
            c.add_working_days(d(2026, 11, 21), 0).unwrap(),
            d(2026, 11, 23)
        );
        assert_eq!(
            c.sub_working_days(d(2026, 11, 22), 0).unwrap(),
            d(2026, 11, 20)
        );
        // A working day is itself.
        assert_eq!(
            c.add_working_days(d(2026, 11, 18), 0).unwrap(),
            d(2026, 11, 18)
        );
    }

    #[test]
    fn holidays_are_skipped_across_a_weekend_and_the_year_end() {
        let c = christmas();
        assert!(!c.is_working_day(d(2026, 12, 25)));
        assert!(!c.is_working_day(d(2026, 12, 28)));
        // Thursday 24th plus one: past Friday (holiday), the weekend and Monday (holiday) to Tuesday the 29th.
        assert_eq!(
            c.add_working_days(d(2026, 12, 24), 1).unwrap(),
            d(2026, 12, 29)
        );
        // Across the year end: Wednesday 30th plus three is Monday 2027-01-04.
        assert_eq!(
            c.add_working_days(d(2026, 12, 30), 3).unwrap(),
            d(2027, 1, 4)
        );
        assert_eq!(
            c.sub_working_days(d(2026, 12, 29), 1).unwrap(),
            d(2026, 12, 24)
        );
    }

    #[test]
    fn counting_working_days_between_two_dates() {
        let c = christmas();
        assert_eq!(c.working_days_between(d(2026, 12, 24), d(2026, 12, 29)), 1);
        assert_eq!(c.working_days_between(d(2026, 11, 16), d(2026, 11, 23)), 5);
        assert_eq!(c.working_days_between(d(2026, 11, 23), d(2026, 11, 16)), 0);
        assert_eq!(c.working_days_between(d(2026, 11, 18), d(2026, 11, 18)), 0);
    }

    #[test]
    fn the_settings_are_parsed_and_mistakes_are_refused() {
        // Default.
        assert_eq!(
            WorkingCalendar::parse(None, None).unwrap(),
            WorkingCalendar::default()
        );
        assert_eq!(
            WorkingCalendar::parse(Some("  "), Some("")).unwrap(),
            WorkingCalendar::default()
        );
        // A team that works Saturdays and has Monday off.
        let c = WorkingCalendar::parse(Some("Sun, MON"), None).unwrap();
        assert!(c.is_working_day(d(2026, 11, 21)));
        assert!(!c.is_working_day(d(2026, 11, 23)));
        // Every day worked.
        let every = WorkingCalendar::parse(Some("none"), None).unwrap();
        assert!(every.is_working_day(d(2026, 11, 22)));
        // Mistakes.
        assert!(WorkingCalendar::parse(Some("sat,funday"), None).is_err());
        assert!(WorkingCalendar::parse(None, Some("25/12/2026")).is_err());
        assert!(WorkingCalendar::parse(Some("mon,tue,wed,thu,fri,sat,sun"), None).is_err());
    }

    #[test]
    fn a_calendar_that_never_works_cannot_be_built_so_the_walk_ends() {
        // Six days off and the seventh a holiday every week for a long run would still end: the walk is bounded.
        let holidays =
            (0..2000).filter_map(|i| d(2026, 1, 5).checked_add(jiff::Span::new().weeks(i)).ok());
        let c = WorkingCalendar::new(
            [
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
                Weekday::Saturday,
                Weekday::Sunday,
            ],
            holidays,
        )
        .unwrap();
        assert!(c.add_working_days(d(2026, 1, 1), 1).is_err());
    }

    #[test]
    fn no_holiday_is_named_in_code() {
        let source = include_str!("calendar.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        for name in ["Christmas", "Boxing", "Easter", "bank holiday", "St David"] {
            assert!(!code.contains(name), "{name} is configuration, not code");
        }
    }
}
