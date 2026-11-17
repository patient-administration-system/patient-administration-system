//! The environment registry and its accessors live in the domain crate
//! (`patient_administration_system_domain::config::env`), so the business rules there read their settings the
//! same way; this module re-exports them and adds the start-up check, which needs the server's own parsers.

pub use patient_administration_system_domain::config::env::*;

use crate::{Error, Result};

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
    if let Some(raw) = text(crate::suspicions::profile::SETTING)
        && let Err(e) = crate::suspicions::Profile::load(&raw)
    {
        problems.push(e.to_string());
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
