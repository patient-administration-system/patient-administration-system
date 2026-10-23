//! The one place the process environment is read (QUAL-6).
//!
//! Every variable that is not plain text is listed in [`VARS`] with its kind, default and bounds, so a
//! default is written once and a typo in a number or a switch is found at start-up, not after the first
//! request that needs it. [`validate`] checks every one that is set and reports all the problems together;
//! `main` calls it before anything else. The accessors ([`flag`], [`int`], [`float`], [`text`]) are what the
//! rest of the server calls; at run time an unusable value falls back to the default (the server is already
//! up, and `validate` has already said so once).
//!
//! A switch accepts `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`, in any case. Before QUAL-6 most switches
//! accepted only the exact text `true`, so `TRUE` or `1` silently left them off.
//!
//! `tests/qual6_settings_test.rs` fails if any other file under `src/` (outside tests and `src/bin`) reads
//! the environment.

use crate::{Error, Result};

/// What a variable holds and what is acceptable.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Flag {
        default: bool,
    },
    Int {
        default: i64,
        min: i64,
        max: i64,
    },
    Float {
        default: f64,
        min: f64,
        max: f64,
    },
    /// A JSON document.
    Json,
    /// An IANA time zone name such as `Europe/London`.
    TimeZone,
}

#[derive(Debug, Clone, Copy)]
pub struct Var {
    pub name: &'static str,
    pub kind: Kind,
}

const fn flag_var(name: &'static str) -> Var {
    Var {
        name,
        kind: Kind::Flag { default: false },
    }
}

const fn int_var(name: &'static str, default: i64, min: i64, max: i64) -> Var {
    Var {
        name,
        kind: Kind::Int { default, min, max },
    }
}

/// Every non-text variable the server reads. Text variables (URLs, secrets, names, the quiet-hours window and
/// the reminder steps) are read with [`text`]; the last two are checked by [`validate`] through their own
/// parsers.
pub const VARS: &[Var] = &[
    // Switches.
    flag_var("PAS_NOSHOW_SCORE"),
    flag_var("PAS_NOSHOW_SHOW_PERCENT"),
    flag_var("PAS_BACKFILL_OFFERS"),
    flag_var("PAS_WHATSAPP_REQUIRE_OPTIN"),
    flag_var("PAS_REMINDER_LADDER"),
    flag_var("PAS_REMINDERS_ENABLED"),
    flag_var("PAS_ALLOW_LEGACY_API_TOKEN"),
    flag_var("PAS_REQUIRE_AUTH"),
    // The server.
    int_var("SERVER_PORT", 8080, 1, 65_535),
    int_var("PAS_OUTBOX_MAX_RETRIES", 10, 0, 1_000),
    // 0 turns the limiter off (documented in operations/deployment.md), so 0 must stay valid.
    int_var("PAS_RATE_LIMIT_RPM", 600, 0, 1_000_000),
    int_var("PAS_RATE_LIMIT_BURST", 60, 0, 1_000_000),
    int_var("PAS_WEBHOOK_TIMEOUT_SECS", 10, 1, 3_600),
    int_var("UPSTREAM_TIMEOUT_SECS", 10, 1, 600),
    int_var("PAS_PASETO_KEYS_REFRESH_SECS", 300, 30, 86_400),
    // Timers and windows.
    int_var("PAS_TOLERANCE_CHECK_MINUTES", 60, 0, 10_080),
    int_var("PAS_FOLLOW_UP_HOURS", 48, 1, 8_760),
    int_var("PAS_OFFER_MINUTES", 30, 1, 10_080),
    int_var("PAS_REMINDER_MAX_TOUCHES", 4, 1, 50),
    int_var("PAS_REBOOK_HORIZON_DAYS", 42, 1, 365),
    int_var("PAS_REBOOK_MIN_NOTICE_HOURS", 24, 1, 8_760),
    int_var("PAS_REBOOK_OFFER_MINUTES", 120, 1, 10_080),
    int_var("PAS_CHECK_WEEKS", 26, 1, 520),
    int_var("PAS_CHECK_REPEAT_DAYS", 90, 1, 3_650),
    // The no-show score.
    int_var("PAS_NOSHOW_CALL_CAPACITY", 10, 0, 500),
    int_var("PAS_NOSHOW_SHADOW_MIN_DAYS", 28, 0, 365),
    int_var("PAS_NOSHOW_CALL_HORIZON_DAYS", 7, 1, 60),
    Var {
        name: "PAS_NOSHOW_MEDIUM",
        kind: Kind::Float {
            default: 0.40,
            min: 0.0,
            max: 1.0,
        },
    },
    Var {
        name: "PAS_NOSHOW_WEIGHTS",
        kind: Kind::Json,
    },
    // Minutes to first clinician contact for each triage category (NEW-23); checked by `validate`.
    Var {
        name: "PAS_TRIAGE_TARGETS",
        kind: Kind::Json,
    },
    // Analytics: the floor for k is 5 by design (see `governance/analytics-differencing.md`).
    int_var("PAS_ANALYTICS_QUERY_BUDGET", 100, 1, 10_000),
    int_var("PAS_ANALYTICS_K", 10, 5, 1_000_000),
    Var {
        name: "PAS_TIMEZONE",
        kind: Kind::TimeZone,
    },
];

fn var(name: &str) -> Option<&'static Var> {
    VARS.iter().find(|v| v.name == name)
}

/// The value as set, or `None` when unset, blank or not valid text.
pub fn text(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`, in any case.
pub fn parse_flag(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Look a registered name up, or say so loudly (a programming error `tests/qual6_settings_test.rs` also checks).
fn registered(name: &str) -> Option<&'static Var> {
    let v = var(name);
    if v.is_none() {
        tracing::error!(name, "environment variable is not in config::env::VARS");
    }
    v
}

/// A switch. Unset, blank or unreadable is the default (off).
pub fn flag(name: &str) -> bool {
    let default = match registered(name).map(|v| v.kind) {
        Some(Kind::Flag { default }) => default,
        _ => false,
    };
    match text(name) {
        None => default,
        Some(raw) => parse_flag(&raw).unwrap_or(default),
    }
}

/// A whole number within its bounds; unset, unreadable or out of range is the default.
pub fn int(name: &str) -> i64 {
    let Some(Kind::Int { default, min, max }) = registered(name).map(|v| v.kind) else {
        return 0;
    };
    text(name)
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|n| (min..=max).contains(n))
        .unwrap_or(default)
}

/// A number within its bounds; unset, unreadable or out of range is the default.
pub fn float(name: &str) -> f64 {
    let Some(Kind::Float { default, min, max }) = registered(name).map(|v| v.kind) else {
        return 0.0;
    };
    text(name)
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|n| (min..=max).contains(n))
        .unwrap_or(default)
}

/// What is wrong with one variable's value, if anything.
fn problem(v: &Var, raw: &str) -> Option<String> {
    let raw = raw.trim();
    match v.kind {
        Kind::Flag { .. } => parse_flag(raw)
            .is_none()
            .then(|| "must be true, false, 1, 0, yes, no, on or off".to_string()),
        Kind::Int { default, min, max } => match raw.parse::<i64>() {
            Ok(n) if (min..=max).contains(&n) => None,
            Ok(_) => Some(format!(
                "must be between {min} and {max} (default {default})"
            )),
            Err(_) => Some(format!(
                "must be a whole number between {min} and {max} (default {default})"
            )),
        },
        Kind::Float { default, min, max } => match raw.parse::<f64>() {
            Ok(n) if (min..=max).contains(&n) => None,
            _ => Some(format!(
                "must be a number between {min} and {max} (default {default})"
            )),
        },
        Kind::Json => serde_json::from_str::<serde_json::Value>(raw)
            .is_err()
            .then(|| "must be valid JSON".to_string()),
        Kind::TimeZone => raw
            .parse::<chrono_tz::Tz>()
            .is_err()
            .then(|| "must be a time zone name such as Europe/London".to_string()),
    }
}

/// Check every registered variable that is set, and the quiet-hours window and reminder steps, and report all
/// the problems in one error. Unset and blank are fine: they mean the default.
pub fn validate() -> Result<()> {
    let mut problems: Vec<String> = Vec::new();
    for v in VARS {
        if let Some(raw) = text(v.name)
            && let Some(why) = problem(v, &raw)
        {
            problems.push(format!("{}={raw:?} {why}", v.name));
        }
    }
    if let Err(e) = crate::communication::consent::QuietHours::from_env() {
        problems.push(format!("PAS_QUIET_HOURS: {e}"));
    }
    if let Some(raw) = text("PAS_TRIAGE_TARGETS")
        && let Err(e) = crate::triage::targets::Targets::from_json(&raw)
    {
        problems.push(e.to_string());
    }
    if let Some(steps) = text("PAS_REMINDER_STEPS")
        && let Err(e) = crate::communication::ladder::parse_steps(&steps)
    {
        problems.push(format!("PAS_REMINDER_STEPS: {e}"));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Error::config(format!(
            "invalid environment: {}",
            problems.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_accept_the_usual_spellings_and_nothing_else() {
        for yes in ["true", "TRUE", " True ", "1", "yes", "ON"] {
            assert_eq!(parse_flag(yes), Some(true), "{yes}");
        }
        for no in ["false", "False", "0", "no", "off"] {
            assert_eq!(parse_flag(no), Some(false), "{no}");
        }
        for bad in ["", "maybe", "2", "tru", "y"] {
            assert_eq!(parse_flag(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_bad_number_is_a_clear_problem() {
        let v = var("PAS_TOLERANCE_CHECK_MINUTES").expect("registered");
        let why = problem(v, "abc").expect("a problem");
        assert!(why.contains("whole number between 0 and 10080"), "{why}");
        assert!(problem(v, "0").is_none(), "0 turns the check off");
        assert!(problem(v, "10081").is_some());
        assert!(problem(v, "-1").is_some());
    }

    /// A zero that the deployment guide documents as "off" must pass the start-up check (it did not at first:
    /// QUAL-6 shipped `PAS_RATE_LIMIT_RPM` with a minimum of 1).
    #[test]
    fn documented_zeros_are_valid() {
        for name in [
            "PAS_RATE_LIMIT_RPM",
            "PAS_RATE_LIMIT_BURST",
            "PAS_TOLERANCE_CHECK_MINUTES",
            "PAS_OUTBOX_MAX_RETRIES",
        ] {
            let v = var(name).expect("registered");
            assert!(problem(v, "0").is_none(), "{name}=0 must be accepted");
        }
        let rpm = var("PAS_RATE_LIMIT_RPM").expect("registered");
        assert!(problem(rpm, "-1").is_some());
    }

    #[test]
    fn each_kind_checks_its_own_shape() {
        let medium = var("PAS_NOSHOW_MEDIUM").expect("registered");
        assert!(problem(medium, "0.4").is_none());
        assert!(problem(medium, "1.5").is_some());
        assert!(problem(medium, "high").is_some());
        let weights = var("PAS_NOSHOW_WEIGHTS").expect("registered");
        assert!(problem(weights, "{\"lead_time\": 0.1}").is_none());
        assert!(problem(weights, "{lead_time").is_some());
        let zone = var("PAS_TIMEZONE").expect("registered");
        assert!(problem(zone, "Europe/London").is_none());
        assert!(problem(zone, "London").is_some());
        let switch = var("PAS_REQUIRE_AUTH").expect("registered");
        assert!(problem(switch, "yes").is_none());
        assert!(problem(switch, "sure").is_some());
    }

    #[test]
    fn names_are_unique_and_defaults_are_inside_their_bounds() {
        let mut seen = std::collections::BTreeSet::new();
        for v in VARS {
            assert!(seen.insert(v.name), "{} is listed twice", v.name);
            match v.kind {
                Kind::Int { default, min, max } => {
                    assert!(min <= default && default <= max, "{}", v.name)
                }
                Kind::Float { default, min, max } => {
                    assert!(min <= default && default <= max, "{}", v.name)
                }
                _ => {}
            }
        }
    }
}
