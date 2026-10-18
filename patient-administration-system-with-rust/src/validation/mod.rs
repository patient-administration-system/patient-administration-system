//! validation
//!
//! Cross-cutting validators for domain types. These complement the
//! constructor-level invariants in `src/models/` with rules that depend on
//! the outside world (current time) or on relationships between fields.
//!
//! Each validator returns `Err(Error::Validation(_))` for the first rule it
//! finds violated; callers should treat them as fail-fast.

use crate::models::appointment::Appointment;
use crate::models::billing::Charge;
use crate::models::{Address, ContactPoint, ContactPointSystem};
use crate::{Error, Result};

// `validate_patient` + `validate_identifier` + the national-identifier
// validators (`validate_nhs_number`, `validate_nir`, `validate_tsi`,
// `validate_ihi`, `validate_hcn`) were removed in v0.44.0 Option C —
// person validation lives in `person-service` now and the PAS
// proxy handlers forward the upstream's `400 + invalid` response.

/// Validate an [`Address`].
///
/// At least one of `city`, `postal_code`, or `country` must be populated;
/// an address with only `line1`/`line2`/`state` cannot be routed.
pub fn validate_address(a: &Address) -> Result<()> {
    if a.city.is_none() && a.postal_code.is_none() && a.country.is_none() {
        return Err(Error::validation(
            "address requires at least city, postal_code, or country",
        ));
    }
    Ok(())
}

/// Validate a [`ContactPoint`].
///
/// - Emails must contain both `@` and `.`.
/// - Phone/SMS/Fax values must have between 7 and 15 ASCII digits.
/// - All other systems pass without further checks.
pub fn validate_contact_point(c: &ContactPoint) -> Result<()> {
    match c.system {
        ContactPointSystem::Email =>
        {
            #[allow(clippy::collapsible_match)]
            if !c.value.contains('@') || !c.value.contains('.') {
                return Err(Error::validation(format!("invalid email: {}", c.value)));
            }
        }
        ContactPointSystem::Phone | ContactPointSystem::Sms | ContactPointSystem::Fax => {
            let digits = c.value.chars().filter(|x| x.is_ascii_digit()).count();
            if !(7..=15).contains(&digits) {
                return Err(Error::validation(format!(
                    "phone digits out of range: {}",
                    c.value
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validate an [`Appointment`]'s time range: `start_datetime` must be strictly
/// before `end_datetime`. Zero-length and inverted ranges are rejected.
pub fn validate_appointment(a: &Appointment) -> Result<()> {
    if a.start_datetime >= a.end_datetime {
        return Err(Error::validation("appointment start must be before end"));
    }
    Ok(())
}

/// Validate a [`Charge`]: the monetary amount must be non-negative.
pub fn validate_charge(c: &Charge) -> Result<()> {
    if c.amount.amount.is_sign_negative() {
        return Err(Error::validation("charge amount must be non-negative"));
    }
    Ok(())
}
// (national-identifier validators removed in v0.44.0 Option C)
pub fn normalize_phone(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 1);
    if s.trim_start().starts_with('+') {
        out.push('+');
    }
    for c in s.chars().filter(|c| c.is_ascii_digit()) {
        out.push(c);
    }
    out
}
