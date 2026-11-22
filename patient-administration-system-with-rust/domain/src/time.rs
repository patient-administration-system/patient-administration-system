//! Time helpers on jiff (JIFF-1).
//!
//! The PAS keeps instants as [`Timestamp`] (UTC, as Postgres `timestamptz` stores them) and calendar dates as
//! [`Date`]. These helpers name the few conversions the code makes again and again, so each is written once:
//! a whole number of days as a duration, an instant's date in UTC or in a time zone, and an instant built from
//! its parts (mostly for tests).
//!
//! A day here is 24 hours, as `chrono::Duration::days` was: "seven days after the booking" is 168 hours, not
//! seven calendar days across a clock change. Calendar arithmetic on dates (months, working days) uses
//! [`Date`] and [`jiff::Span`] instead.

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

/// `n` days of 24 hours.
pub fn days(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n.saturating_mul(24))
}

/// The whole days in `d`, truncated towards zero (as `chrono::Duration::num_days` was).
pub fn whole_days(d: SignedDuration) -> i64 {
    d.as_hours() / 24
}

/// The date of `at` in UTC.
pub fn utc_date(at: Timestamp) -> Date {
    at.to_zoned_datetime(TimeZone::UTC).date()
}

/// The date of `at` in `tz`.
pub fn date_in(at: Timestamp, tz: &TimeZone) -> Date {
    at.to_zoned_datetime(tz.clone()).date()
}

/// Midnight UTC at the start of `date`.
pub fn start_of_utc_day(date: Date) -> Timestamp {
    date.to_zoned_datetime(TimeZone::UTC)
        .map(|z| z.timestamp())
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

/// The instant `y-m-d h:mi:s` UTC. Out-of-range parts give the Unix epoch, so a typo in a test fails loudly
/// rather than panicking deep in a fixture.
pub fn utc(y: i16, m: i8, d: i8, h: i8, mi: i8, s: i8) -> Timestamp {
    jiff::civil::DateTime::new(y, m, d, h, mi, s, 0)
        .and_then(|dt| dt.to_zoned_datetime(TimeZone::UTC))
        .map(|z| z.timestamp())
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

/// The whole days from `from` to `to` (negative when `to` is earlier), as `chrono`'s `NaiveDate - NaiveDate`
/// was.
pub fn day_count(from: Date, to: Date) -> i64 {
    to.since(from).map(|s| i64::from(s.get_days())).unwrap_or(0)
}

/// Whole years from `born` to `on`: an age. `None` when `born` is after `on`.
pub fn years_between(born: Date, on: Date) -> Option<i64> {
    if born > on {
        return None;
    }
    on.since((jiff::Unit::Year, born))
        .ok()
        .map(|s| i64::from(s.get_years()))
}

/// `at` minus `months` calendar months in UTC, clamped like [`add_months_utc`].
pub fn sub_months_utc(at: Timestamp, months: u32) -> Option<Timestamp> {
    at.to_zoned_datetime(TimeZone::UTC)
        .checked_sub(jiff::Span::new().months(i64::from(months)))
        .ok()
        .map(|z| z.timestamp())
}

/// The name of `tz` for SQL `AT TIME ZONE` and for clients: its IANA name, or `UTC`.
pub fn zone_name(tz: &TimeZone) -> String {
    tz.iana_name().unwrap_or("UTC").to_string()
}

/// The time zone named `name`, from the bundled database.
pub fn zone(name: &str) -> Option<TimeZone> {
    TimeZone::get(name).ok()
}

/// Monday is 0 and Sunday 6 (as `chrono::Weekday::num_days_from_monday`).
pub fn monday_index(w: Weekday) -> usize {
    usize::try_from(w.to_monday_zero_offset()).unwrap_or(0)
}

/// The three-letter English name a weekday is stored and sent as (`Mon` to `Sun`), the form `chrono` used.
pub fn weekday_name(w: Weekday) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .get(monday_index(w))
        .copied()
        .unwrap_or("Mon")
}

/// The weekday for a three-letter English name, or a full one, in any case.
pub fn weekday_from_name(s: &str) -> Option<Weekday> {
    let lower = s.trim().to_ascii_lowercase();
    Some(match lower.get(..3)? {
        "mon" => Weekday::Monday,
        "tue" => Weekday::Tuesday,
        "wed" => Weekday::Wednesday,
        "thu" => Weekday::Thursday,
        "fri" => Weekday::Friday,
        "sat" => Weekday::Saturday,
        "sun" => Weekday::Sunday,
        _ => return None,
    })
}

/// `at` plus `months` calendar months in UTC, clamped to the end of a shorter month (31 January plus one month
/// is the last day of February), as `chrono`'s `Months` was; `None` past the end of the supported range.
pub fn add_months_utc(at: Timestamp, months: u32) -> Option<Timestamp> {
    at.to_zoned_datetime(TimeZone::UTC)
        .checked_add(jiff::Span::new().months(i64::from(months)))
        .ok()
        .map(|z| z.timestamp())
}

/// Serde for an optional list of weekdays as `["Mon", "Wed"]`.
pub mod weekdays_opt_serde {
    use jiff::civil::Weekday;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(days: &Option<Vec<Weekday>>, s: S) -> Result<S::Ok, S::Error> {
        match days {
            Some(d) => super::weekdays_serde::serialize(d, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<Weekday>>, D::Error> {
        match Option::<Vec<String>>::deserialize(d)? {
            None => Ok(None),
            Some(names) => names
                .iter()
                .map(|n| {
                    super::weekday_from_name(n)
                        .ok_or_else(|| serde::de::Error::custom(format!("{n:?} is not a weekday")))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
        }
    }
}

/// Serde for a list of weekdays as `["Mon", "Wed"]`, unchanged from the chrono form on the wire.
pub mod weekdays_serde {
    use jiff::civil::Weekday;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(days: &[Weekday], s: S) -> Result<S::Ok, S::Error> {
        days.iter()
            .map(|d| super::weekday_name(*d))
            .collect::<Vec<_>>()
            .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Weekday>, D::Error> {
        Vec::<String>::deserialize(d)?
            .iter()
            .map(|n| {
                super::weekday_from_name(n)
                    .ok_or_else(|| serde::de::Error::custom(format!("{n:?} is not a weekday")))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_24_hours_and_whole_days_truncate() {
        assert_eq!(days(2), SignedDuration::from_hours(48));
        assert_eq!(whole_days(SignedDuration::from_hours(47)), 1);
        assert_eq!(whole_days(SignedDuration::from_hours(-47)), -1);
    }

    #[test]
    fn dates_in_utc_and_in_a_zone() {
        let late = utc(2026, 7, 7, 23, 30, 0);
        assert_eq!(utc_date(late), jiff::civil::date(2026, 7, 7));
        let london = zone(crate::config::env::DEFAULT_TIME_ZONE).unwrap();
        assert_eq!(
            date_in(late, &london),
            jiff::civil::date(2026, 7, 8),
            "BST is UTC+1"
        );
        assert_eq!(
            start_of_utc_day(jiff::civil::date(2026, 7, 7)),
            utc(2026, 7, 7, 0, 0, 0)
        );
    }

    #[test]
    fn weekday_names_round_trip() {
        for w in [Weekday::Monday, Weekday::Saturday, Weekday::Sunday] {
            assert_eq!(weekday_from_name(weekday_name(w)), Some(w));
        }
        assert_eq!(weekday_from_name("WEDNESDAY"), Some(Weekday::Wednesday));
        assert_eq!(weekday_from_name("x"), None);
        assert_eq!(monday_index(Weekday::Sunday), 6);
    }

    #[test]
    fn day_count_and_age() {
        let d = jiff::civil::date;
        assert_eq!(day_count(d(2026, 1, 1), d(2026, 1, 31)), 30);
        assert_eq!(day_count(d(2026, 1, 31), d(2026, 1, 1)), -30);
        assert_eq!(years_between(d(2000, 6, 15), d(2026, 6, 14)), Some(25));
        assert_eq!(years_between(d(2000, 6, 15), d(2026, 6, 15)), Some(26));
        assert_eq!(years_between(d(2026, 6, 15), d(2026, 6, 15)), Some(0));
        assert_eq!(years_between(d(2026, 6, 16), d(2026, 6, 15)), None);
    }

    #[test]
    fn months_clamp_to_the_end_of_a_shorter_month() {
        let jan31 = utc(2026, 1, 31, 10, 0, 0);
        assert_eq!(add_months_utc(jan31, 1), Some(utc(2026, 2, 28, 10, 0, 0)));
        assert_eq!(add_months_utc(jan31, 0), Some(jan31));
        let mar31 = utc(2026, 3, 31, 10, 0, 0);
        assert_eq!(sub_months_utc(mar31, 1), Some(utc(2026, 2, 28, 10, 0, 0)));
        assert_eq!(sub_months_utc(mar31, 3), Some(utc(2025, 12, 31, 10, 0, 0)));
    }

    #[test]
    fn zones_by_name() {
        assert_eq!(zone_name(&zone("Europe/Paris").unwrap()), "Europe/Paris");
        assert_eq!(zone_name(&TimeZone::UTC), "UTC");
        assert!(zone("Not/AZone").is_none());
    }

    #[test]
    fn weekday_names_in_full_and_short_and_invalid() {
        assert_eq!(weekday_from_name(" tue "), Some(Weekday::Tuesday));
        for (n, w) in [
            ("mon", Weekday::Monday),
            ("thu", Weekday::Thursday),
            ("fri", Weekday::Friday),
            ("sat", Weekday::Saturday),
            ("sun", Weekday::Sunday),
        ] {
            assert_eq!(weekday_from_name(n), Some(w));
        }
        assert_eq!(weekday_from_name("xyz"), None);
        assert_eq!(weekday_from_name(""), None);
        assert_eq!(weekday_name(Weekday::Monday), "Mon");
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct Days {
        #[serde(with = "weekdays_serde")]
        all: Vec<Weekday>,
        #[serde(with = "weekdays_opt_serde", default)]
        some: Option<Vec<Weekday>>,
    }

    #[test]
    fn weekday_lists_serialise_as_names() {
        let v = Days {
            all: vec![Weekday::Monday, Weekday::Friday],
            some: Some(vec![Weekday::Sunday]),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"all":["Mon","Fri"],"some":["Sun"]}"#);
        assert_eq!(serde_json::from_str::<Days>(&json).unwrap(), v);
        let none = Days {
            all: vec![],
            some: None,
        };
        let json = serde_json::to_string(&none).unwrap();
        assert_eq!(json, r#"{"all":[],"some":null}"#);
        assert_eq!(serde_json::from_str::<Days>(&json).unwrap(), none);
        assert!(serde_json::from_str::<Days>(r#"{"all":["Funday"]}"#).is_err());
        assert!(serde_json::from_str::<Days>(r#"{"all":[],"some":["Funday"]}"#).is_err());
    }
}
