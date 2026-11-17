//! The time-to-first-clinician-contact target for each triage category (NEW-23).
//!
//! The usual Manchester Triage System figures are 0, 10, 60, 120 and 240 minutes, but sources differ and a
//! board may set its own, so they are configuration: `PAS_TRIAGE_TARGETS` is a JSON object of category to
//! whole minutes, and a category left out keeps its default. A target is a measure of the service, never an
//! instruction about who is seen next (NFR-11).

use chrono::{DateTime, Duration, Utc};

use super::CATEGORIES;
use crate::{Error, Result};

/// The default minutes for each category, in the order of [`CATEGORIES`].
pub const DEFAULT_MINUTES: [i64; 5] = [0, 10, 60, 120, 240];

/// The longest target accepted: a week.
const MAX_MINUTES: i64 = 10_080;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Targets([i64; 5]);

impl Default for Targets {
    fn default() -> Self {
        Self(DEFAULT_MINUTES)
    }
}

impl Targets {
    /// The defaults with each category named in `json` replaced. Unknown categories, non-integers and
    /// negative or absurd values are refused, so a typo cannot quietly leave a default in force.
    pub fn from_json(json: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|_| Error::config("PAS_TRIAGE_TARGETS must be valid JSON"))?;
        let map = value.as_object().ok_or_else(|| {
            Error::config("PAS_TRIAGE_TARGETS must be an object of category to minutes")
        })?;
        let mut minutes = DEFAULT_MINUTES;
        for (key, v) in map {
            let i = CATEGORIES.iter().position(|c| c == key).ok_or_else(|| {
                Error::config(format!(
                    "PAS_TRIAGE_TARGETS: unknown category {key:?}; use {}",
                    CATEGORIES.join(", ")
                ))
            })?;
            let m = v
                .as_i64()
                .filter(|m| (0..=MAX_MINUTES).contains(m))
                .ok_or_else(|| {
                    Error::config(format!(
                        "PAS_TRIAGE_TARGETS: {key} must be whole minutes between 0 and {MAX_MINUTES}"
                    ))
                })?;
            minutes[i] = m;
        }
        Ok(Self(minutes))
    }

    /// From `PAS_TRIAGE_TARGETS`, or the defaults when it is unset or invalid (`config::env::validate`
    /// reports an invalid value at start-up).
    pub fn from_env() -> Self {
        crate::config::env::text("PAS_TRIAGE_TARGETS")
            .and_then(|raw| Self::from_json(&raw).ok())
            .unwrap_or_default()
    }

    /// The target for a category, or `None` for a name that is not one.
    pub fn minutes(&self, category: &str) -> Option<i64> {
        CATEGORIES
            .iter()
            .position(|c| *c == category)
            .map(|i| self.0[i])
    }
}

/// Where an attendance stands against its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Seen within the target.
    SeenWithin,
    /// Seen, but after the target.
    SeenPast,
    /// Not yet seen, and still within the target.
    WaitingWithin,
    /// Not yet seen, and past the target.
    WaitingPast,
}

impl Standing {
    pub fn is_past(self) -> bool {
        matches!(self, Self::SeenPast | Self::WaitingPast)
    }
}

/// Judge an attendance. Exactly the target is within it; a second more is past. An attendance not yet seen
/// is measured to `now`.
pub fn standing(
    arrival: DateTime<Utc>,
    first_contact: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    target_minutes: i64,
) -> Standing {
    let limit = Duration::minutes(target_minutes);
    match first_contact {
        Some(seen) if seen - arrival <= limit => Standing::SeenWithin,
        Some(_) => Standing::SeenPast,
        None if now - arrival <= limit => Standing::WaitingWithin,
        None => Standing::WaitingPast,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap()
    }

    #[test]
    fn nine_ten_and_eleven_minutes_against_a_ten_minute_target() {
        let seen = |m: i64| standing(t0(), Some(t0() + Duration::minutes(m)), t0(), 10);
        assert_eq!(seen(9), Standing::SeenWithin);
        assert_eq!(seen(10), Standing::SeenWithin);
        assert_eq!(seen(11), Standing::SeenPast);
        let one_second = standing(
            t0(),
            Some(t0() + Duration::minutes(10) + Duration::seconds(1)),
            t0(),
            10,
        );
        assert_eq!(one_second, Standing::SeenPast);
    }

    #[test]
    fn someone_not_yet_seen_is_measured_to_now() {
        let at = |m: i64| standing(t0(), None, t0() + Duration::minutes(m), 10);
        assert_eq!(at(9), Standing::WaitingWithin);
        assert_eq!(at(10), Standing::WaitingWithin);
        assert_eq!(at(11), Standing::WaitingPast);
        assert!(at(11).is_past() && !at(9).is_past());
    }

    #[test]
    fn an_immediate_target_of_zero_allows_no_wait() {
        assert_eq!(standing(t0(), Some(t0()), t0(), 0), Standing::SeenWithin);
        assert_eq!(
            standing(t0(), Some(t0() + Duration::seconds(1)), t0(), 0),
            Standing::SeenPast
        );
    }

    #[test]
    fn the_defaults_are_0_10_60_120_240() {
        let t = Targets::default();
        let got: Vec<_> = CATEGORIES.iter().map(|c| t.minutes(c).unwrap()).collect();
        assert_eq!(got, vec![0, 10, 60, 120, 240]);
        assert_eq!(t.minutes("red"), None);
    }

    #[test]
    fn a_configured_target_replaces_the_default_and_the_rest_stay() {
        let t = Targets::from_json(r#"{"very_urgent": 15, "non_urgent": 180}"#).unwrap();
        assert_eq!(t.minutes("very_urgent"), Some(15));
        assert_eq!(t.minutes("non_urgent"), Some(180));
        assert_eq!(t.minutes("immediate"), Some(0));
        assert_eq!(t.minutes("urgent"), Some(60));
    }

    #[test]
    fn a_bad_configuration_is_refused_not_ignored() {
        for bad in [
            "[]",
            "nope",
            r#"{"red": 10}"#,
            r#"{"urgent": -1}"#,
            r#"{"urgent": 1.5}"#,
            r#"{"urgent": "60"}"#,
            r#"{"urgent": 99999}"#,
        ] {
            assert!(Targets::from_json(bad).is_err(), "{bad}");
        }
    }
}
