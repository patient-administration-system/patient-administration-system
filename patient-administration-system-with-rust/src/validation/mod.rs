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
// (national-identifier validators removed in v0.44.0 Option C)
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

/// The FHIR identifier system for an NHS number.
pub const NHS_NUMBER_SYSTEM: &str = "https://fhir.nhs.uk/Id/nhs-number";

/// Check an NHS number (DTAC-8): ten digits whose last is the modulus-11 check digit of the first nine
/// (weights 10 down to 2; `11 - (sum mod 11)`, where 11 becomes 0 and 10 makes the number invalid). Spaces and
/// hyphens are allowed in the input. Returns the ten digits. It checks the number is well formed; it does not
/// say the number is issued or belongs to this patient (that needs a PDS trace, which the PAS does not do:
/// see `governance/pds-decision.md`).
pub fn validate_nhs_number(raw: &str) -> Result<String> {
    let digits: String = raw.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    if digits.len() != 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::validation("an NHS number is ten digits"));
    }
    let d: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    let sum: u32 = d[..9].iter().zip((2..=10).rev()).map(|(x, w)| x * w).sum();
    let check = match 11 - (sum % 11) {
        11 => 0,
        10 => return Err(Error::validation("that is not a valid NHS number")),
        c => c,
    };
    if check != d[9] {
        return Err(Error::validation(
            "that is not a valid NHS number (the check digit does not match)",
        ));
    }
    Ok(digits)
}

/// The NHS number written the usual way, `943 476 5919`. The input must already be valid.
pub fn format_nhs_number(valid: &str) -> String {
    format!("{} {} {}", &valid[..3], &valid[3..6], &valid[6..])
}

/// Refuse a person body whose `identifiers` carry an NHS number that fails the check (DTAC-8). Other
/// identifiers, and bodies with none, pass untouched.
pub fn check_person_identifiers(fields: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    let Some(ids) = fields.get("identifiers").and_then(|v| v.as_array()) else {
        return Ok(());
    };
    for id in ids {
        if id.get("system").and_then(|v| v.as_str()) == Some(NHS_NUMBER_SYSTEM) {
            let value = id.get("value").and_then(|v| v.as_str()).unwrap_or("");
            validate_nhs_number(value)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod nhs_number_tests {
    use super::*;

    #[test]
    fn test_known_valid_numbers_pass_with_or_without_spaces() {
        // 943 476 5919 is the example in the NHS number specification.
        assert_eq!(validate_nhs_number("9434765919").unwrap(), "9434765919");
        assert_eq!(validate_nhs_number("943 476 5919").unwrap(), "9434765919");
        assert_eq!(validate_nhs_number("943-476-5919").unwrap(), "9434765919");
        assert_eq!(format_nhs_number("9434765919"), "943 476 5919");
    }

    #[test]
    fn test_a_wrong_check_digit_fails() {
        for n in ["9434765918", "9434765910", "1234567890"] {
            assert!(validate_nhs_number(n).is_err(), "{n}");
        }
    }

    #[test]
    fn test_a_check_of_eleven_is_zero_and_a_check_of_ten_is_invalid() {
        // 0000000000: sum 0, 11 - 0 = 11 -> check 0.
        assert!(validate_nhs_number("0000000000").is_ok());
        // 9990000000-ish: find a number whose check would be 10 and see it refused whatever the last digit.
        let mut found = false;
        for n in 0..2000u32 {
            let first9 = format!("{n:09}");
            let d: Vec<u32> = first9.chars().map(|c| c.to_digit(10).unwrap()).collect();
            let sum: u32 = d.iter().zip((2..=10).rev()).map(|(x, w)| x * w).sum();
            if 11 - (sum % 11) == 10 {
                found = true;
                for last in 0..=9 {
                    assert!(validate_nhs_number(&format!("{first9}{last}")).is_err());
                }
                break;
            }
        }
        assert!(found);
    }

    #[test]
    fn test_wrong_length_and_non_digits_fail() {
        for n in [
            "",
            "943476591",
            "94347659190",
            "94347659l9",
            "abcdefghij",
            "943 476 591",
        ] {
            assert!(validate_nhs_number(n).is_err(), "{n}");
        }
    }

    #[test]
    fn test_only_the_nhs_system_is_checked_in_a_person_body() {
        let bad = serde_json::json!({ "identifiers": [{ "system": NHS_NUMBER_SYSTEM, "value": "9434765918" }] });
        assert!(check_person_identifiers(bad.as_object().unwrap()).is_err());
        let good = serde_json::json!({ "identifiers": [{ "system": NHS_NUMBER_SYSTEM, "value": "943 476 5919" }] });
        assert!(check_person_identifiers(good.as_object().unwrap()).is_ok());
        let other = serde_json::json!({ "identifiers": [{ "system": "https://example.test/mrn", "value": "not an nhs number" }] });
        assert!(check_person_identifiers(other.as_object().unwrap()).is_ok());
        assert!(check_person_identifiers(&serde_json::Map::new()).is_ok());
    }
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
