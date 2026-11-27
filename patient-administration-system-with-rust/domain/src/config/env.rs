//! The one place the process environment is read (QUAL-6).
//!
//! Every variable that is not plain text is listed in [`VARS`] with its kind, default and bounds, so a
//! default is written once and a typo in a number or a switch is found at start-up, not after the first
//! request that needs it. The server's `config::env::validate` checks every one that is set (with [`problem`])
//! and reports all the problems together; `main` calls it before anything else. The accessors ([`flag`], [`int`], [`float`], [`text`]) are what the
//! rest of the server calls. An unset or blank variable is its declared default; one that is set to a value
//! that cannot be used is an error, never the default (RCA-IN-2, RCA-IN-3).
//!
//! A switch accepts `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`, in any case. Before QUAL-6 most switches
//! accepted only the exact text `true`, so `TRUE` or `1` silently left them off.
//!
//! `tests/qual6_settings_test.rs` fails if any other file under `src/` (outside tests and `src/bin`) reads
//! the environment.

/// The service time zone when `PAS_TIMEZONE` is unset: the one place it is named (QUAL-40 counts it once).
pub const DEFAULT_TIME_ZONE: &str = "Europe/London";

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
    /// Free text whose meaning is checked by its own reader in [`validate`].
    Text,
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
    flag_var("PAS_REQUIRE_UPSTREAMS"),
    // The server.
    int_var("SERVER_PORT", 8080, 1, 65_535),
    int_var("PAS_OUTBOX_MAX_RETRIES", 10, 0, 1_000),
    // 0 turns the limiter off (documented in operations/deployment.md), so 0 must stay valid.
    int_var("PAS_RATE_LIMIT_RPM", 600, 0, 1_000_000),
    // 401s from one client address in 15 minutes before it is refused until they end. 0, the default, turns the
    // lockout off: the UI servers call the API for all their users from one address, so their users' 401s add up.
    int_var("LOCKOUT_CLIENT_ADDRESS_AFTER_COUNT", 0, 0, 1_000_000),
    // The old name of LOCKOUT_CLIENT_ADDRESS_AFTER_COUNT, read only when that is not set; the server warns.
    int_var("PAS_AUTH_LOCKOUT_FAILURES", 0, 0, 1_000_000),
    // How long a pre-admission holds a bed, in minutes. 0 (the default) means no expiry, as before OFH-4.
    int_var("PAS_BED_HOLD_MINUTES", 0, 0, 10_080),
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
    int_var("PAS_HOLD_MINUTES", 10, 1, 1_440),
    int_var("PAS_DB_MAX_CONNECTIONS", 10, 1, 1_000),
    // TSC-12: the same change refused past this many in the window (0 turns it off).
    int_var("PAS_LOOP_LIMIT", 10, 0, 1_000_000),
    int_var("PAS_LOOP_WINDOW_SECONDS", 60, 1, 86_400),
    int_var("PAS_HL7_DEDUPE_DAYS", 7, 1, 3_650),
    int_var("PAS_OUTBOX_STALE_SECONDS", 1_800, 60, 604_800),
    int_var("PAS_HL7_IDLE_SECONDS", 30, 1, 86_400),
    int_var("PAS_HL7_MAX_CONNECTIONS", 64, 1, 10_000),
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
    // A built-in profile name or a path; checked by loading it (WPAS-8).
    Var {
        name: "PAS_SUSPICION_PROFILE",
        kind: Kind::Text,
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

/// Look a registered name up. An unregistered name is a programming error
/// (`tests/qual6_settings_test.rs` also checks), reported, not answered with a made-up value.
fn registered(name: &str) -> Result<&'static Var, crate::error::Error> {
    var(name).ok_or_else(|| {
        crate::error::Error::config(format!(
            "{name} is not in config::env::VARS, so it has no declared kind or default"
        ))
    })
}

/// The error for a supplied value that is not usable (RCA-IN-2): names the variable and the rule, not the value.
fn bad(v: &Var, raw: &str) -> crate::error::Error {
    crate::error::Error::config(format!(
        "{} {}",
        v.name,
        problem(v, raw).unwrap_or_else(|| "is not usable".to_string())
    ))
}

/// A switch. Unset or blank is the registered default; a value that is set and not a switch is an error.
pub fn flag(name: &str) -> Result<bool, crate::error::Error> {
    let v = registered(name)?;
    let Kind::Flag { default } = v.kind else {
        return Err(crate::error::Error::config(format!(
            "{name} is not a switch"
        )));
    };
    match text(name) {
        None => Ok(default),
        Some(raw) => parse_flag(&raw).ok_or_else(|| bad(v, &raw)),
    }
}

/// A whole number within its bounds. Unset or blank is the registered default (RCA-IN-3); one that is set
/// and unreadable or out of range is an error (RCA-IN-2), never replaced by the default.
pub fn int(name: &str) -> Result<i64, crate::error::Error> {
    let v = registered(name)?;
    let Kind::Int { default, min, max } = v.kind else {
        return Err(crate::error::Error::config(format!(
            "{name} is not a whole number"
        )));
    };
    match text(name) {
        None => Ok(default),
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(n) if (min..=max).contains(&n) => Ok(n),
            _ => Err(bad(v, &raw)),
        },
    }
}

/// A whole number as the integer type the caller needs (`u64`, `u32`, `usize`...): [`int`], and an error if
/// the registered range does not fit that type, never a clamp or a substitute (RCA-CONV-2).
pub fn count<T: TryFrom<i64>>(name: &str) -> Result<T, crate::error::Error> {
    T::try_from(int(name)?).map_err(|_| {
        crate::error::Error::config(format!("{name} does not fit the type it is read as"))
    })
}

/// A number within its bounds, with the same rules as [`int`].
pub fn float(name: &str) -> Result<f64, crate::error::Error> {
    let v = registered(name)?;
    let Kind::Float { default, min, max } = v.kind else {
        return Err(crate::error::Error::config(format!(
            "{name} is not a number"
        )));
    };
    match text(name) {
        None => Ok(default),
        Some(raw) => match raw.trim().parse::<f64>() {
            Ok(n) if (min..=max).contains(&n) => Ok(n),
            _ => Err(bad(v, &raw)),
        },
    }
}

/// What is wrong with one variable's value, if anything.
pub fn problem(v: &Var, raw: &str) -> Option<String> {
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
        Kind::TimeZone => crate::time::zone(raw)
            .is_none()
            .then(|| format!("must be a time zone name such as {DEFAULT_TIME_ZONE}")),
        Kind::Text => None,
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
            "LOCKOUT_CLIENT_ADDRESS_AFTER_COUNT",
            "PAS_AUTH_LOCKOUT_FAILURES",
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
        assert!(problem(zone, DEFAULT_TIME_ZONE).is_none());
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

    /// Sets one variable for the closure and restores it after. Each test below owns its variable (no other
    /// domain code reads it), so parallel tests do not see each other's values.
    fn with_var<T>(name: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let before = std::env::var(name).ok();
        // SAFETY: test-only, and the variable is private to the calling test.
        unsafe {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        let out = f();
        unsafe {
            match before {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        out
    }

    #[test]
    fn text_is_none_when_unset_or_blank() {
        assert_eq!(
            with_var("PAS_TIMEZONE", None, || text("PAS_TIMEZONE")),
            None
        );
        assert_eq!(
            with_var("PAS_TIMEZONE", Some("  "), || text("PAS_TIMEZONE")),
            None
        );
        assert_eq!(
            with_var("PAS_TIMEZONE", Some("Europe/Paris"), || text(
                "PAS_TIMEZONE"
            )),
            Some("Europe/Paris".to_string())
        );
    }

    #[test]
    fn flag_reads_the_value_and_refuses_a_bad_one() {
        let f = |v| with_var("PAS_NOSHOW_SCORE", v, || flag("PAS_NOSHOW_SCORE"));
        assert!(!f(None).unwrap(), "unset is the default");
        assert!(!f(Some("  ")).unwrap(), "blank is unset");
        assert!(f(Some("yes")).unwrap());
        assert!(!f(Some("off")).unwrap());
        // Set but not a switch: an error that names the variable, never the default (RCA-IN-2).
        let e = f(Some("garbage")).unwrap_err().to_string();
        assert!(e.contains("PAS_NOSHOW_SCORE"), "{e}");
        assert!(!e.contains("garbage"), "the value is not echoed: {e}");
        // Not registered, or not a switch: an error, not a made-up answer.
        assert!(flag("PAS_NOT_A_REAL_SETTING").is_err());
        assert!(flag("PAS_TIMEZONE").is_err());
    }

    #[test]
    fn int_reads_within_bounds_and_refuses_the_rest() {
        let f = |v| with_var("PAS_HL7_DEDUPE_DAYS", v, || int("PAS_HL7_DEDUPE_DAYS"));
        assert_eq!(f(None).unwrap(), 7);
        assert_eq!(f(Some(" 30 ")).unwrap(), 30);
        assert!(f(Some("0")).is_err(), "below the minimum");
        assert!(f(Some("99999")).is_err(), "above the maximum");
        assert!(f(Some("many")).is_err());
        assert!(int("PAS_NOT_A_REAL_SETTING").is_err());
        // A registered variable of another kind is not an int.
        assert!(int("PAS_TIMEZONE").is_err());
        let v = var("PAS_HL7_DEDUPE_DAYS").unwrap();
        assert!(problem(v, "99999").unwrap().contains("between 1 and 3650"));
    }

    #[test]
    fn count_reads_an_int_as_the_type_asked_for() {
        let f = |v| {
            with_var("PAS_HL7_DEDUPE_DAYS", v, || {
                count::<u8>("PAS_HL7_DEDUPE_DAYS")
            })
        };
        assert_eq!(f(None).unwrap(), 7);
        // 3650 is a valid setting but does not fit a u8: an error, not a clamp.
        assert!(f(Some("3650")).is_err());
        assert!(count::<u64>("PAS_NOT_A_REAL_SETTING").is_err());
    }

    #[test]
    fn float_reads_within_bounds_and_refuses_the_rest() {
        let f = |v| with_var("PAS_NOSHOW_MEDIUM", v, || float("PAS_NOSHOW_MEDIUM"));
        assert!((f(None).unwrap() - 0.40).abs() < 1e-9);
        assert!((f(Some("0.75")).unwrap() - 0.75).abs() < 1e-9);
        assert!(f(Some("1.5")).is_err());
        assert!(f(Some("high")).is_err());
        assert!(float("PAS_NOT_A_REAL_SETTING").is_err());
        assert!(float("PAS_TIMEZONE").is_err());
    }

    /// RCA-TEST-8 for every registered setting: the declared minimum and maximum are accepted, one outside
    /// either is refused, and so is text that is not a number. Start-up fails on any of them (`validate`).
    #[test]
    fn every_registered_number_is_accepted_at_its_limits_and_refused_outside_them() {
        let mut checked = 0;
        for v in VARS {
            match v.kind {
                Kind::Int { min, max, .. } => {
                    for ok in [min, max] {
                        assert!(problem(v, &ok.to_string()).is_none(), "{} = {ok}", v.name);
                    }
                    for bad in [min.checked_sub(1), max.checked_add(1)]
                        .into_iter()
                        .flatten()
                    {
                        assert!(problem(v, &bad.to_string()).is_some(), "{} = {bad}", v.name);
                    }
                    for bad in ["", "lots", "1.5", "99999999999999999999"] {
                        assert!(problem(v, bad).is_some(), "{} = {bad:?}", v.name);
                    }
                    checked += 1;
                }
                Kind::Float { min, max, .. } => {
                    for ok in [min, max] {
                        assert!(problem(v, &ok.to_string()).is_none(), "{} = {ok}", v.name);
                    }
                    for bad in ["lots", "", "NaN"] {
                        assert!(problem(v, bad).is_some(), "{} = {bad:?}", v.name);
                    }
                    checked += 1;
                }
                Kind::Flag { .. } => {
                    for bad in ["maybe", "2", ""] {
                        assert!(problem(v, bad).is_some(), "{} = {bad:?}", v.name);
                    }
                    checked += 1;
                }
                _ => {}
            }
        }
        assert!(checked > 20, "only {checked} numeric settings were checked");
    }
}
