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

// Person validation lives in `person-service` (v0.44.0 Option C), and the PAS proxy handlers forward its
// `400 + invalid` response. The identifier checks the PAS still makes before a body leaves are in
// `crate::identifiers`, one scheme per file.

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
            #[allow(
                clippy::collapsible_match,
                reason = "one arm per rule reads more plainly than a collapsed match"
            )]
            if !c.value.contains('@') || !c.value.contains('.') {
                return Err(Error::validation(format!("invalid email: {}", c.value)));
            }
        }
        ContactPointSystem::Phone | ContactPointSystem::Sms | ContactPointSystem::Fax => {
            let digits = c.value.chars().filter(char::is_ascii_digit).count();
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
/// The longest name or role we accept for "who did this" fields.
pub const MAX_PERSON_NAME: usize = 120;

/// The name of the person who did or approved something (`approved_by`, `closed_by`, ...).
///
/// Returns the trimmed name (borrowed from the input). 1 to [`MAX_PERSON_NAME`] characters, no control characters. `message` is the error text for an
/// empty name, written for the field in question; a name that is too long or has control characters gets
/// a shared message. A name made only of spaces is empty: it does not count as a named person.
pub fn named_person<'a>(name: &'a str, message: &str) -> Result<&'a str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::validation(message));
    }
    if name.chars().count() > MAX_PERSON_NAME {
        return Err(Error::validation(format!(
            "a name must be at most {MAX_PERSON_NAME} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(Error::validation(
            "a name must not contain control characters",
        ));
    }
    Ok(name)
}

pub fn normalize_phone(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 1);
    if s.trim_start().starts_with('+') {
        out.push('+');
    }
    for c in s.chars().filter(char::is_ascii_digit) {
        out.push(c);
    }
    out
}

#[cfg(test)]
mod named_person_tests {
    use super::named_person;

    #[test]
    fn named_person_is_trimmed_and_never_blank() {
        assert_eq!(named_person("  Dr Jones ", "m").unwrap(), "Dr Jones");
        for blank in ["", "   ", "\t\n"] {
            let e = named_person(blank, "needs a name").unwrap_err().to_string();
            assert!(e.contains("needs a name"), "{e}");
        }
        assert!(named_person(&"x".repeat(121), "m").is_err());
        assert!(named_person(&"x".repeat(120), "m").is_ok());
        assert!(named_person("a\u{0}b", "m").is_err());
        assert!(named_person("a\nb", "m").is_err());
    }
}
