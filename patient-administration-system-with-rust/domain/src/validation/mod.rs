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

/// The most ids one `?ids=` list may carry.
pub const MAX_ID_LIST: usize = 500;

/// A comma-separated list of ids (`?ids=a,b`) as the ids themselves. Every entry must be an id: one that is not
/// is refused, naming the list and its place, never skipped (RCA-IN-2). The list may not be empty or longer than
/// [`MAX_ID_LIST`].
pub fn uuid_list(figure: &str, text: &str) -> Result<Vec<uuid::Uuid>> {
    let entries: Vec<&str> = text.split(',').map(str::trim).collect();
    if entries.len() > MAX_ID_LIST {
        return Err(Error::validation(format!(
            "{figure} may carry at most {MAX_ID_LIST} ids"
        )));
    }
    let mut ids = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        // The list is at most MAX_ID_LIST long, so the place always fits.
        let place = i.checked_add(1).unwrap_or(MAX_ID_LIST);
        match entry.parse::<uuid::Uuid>() {
            Ok(id) => ids.push(id),
            Err(_) => {
                return Err(Error::validation(format!(
                    "{figure}: entry {place} is not an id"
                )));
            }
        }
    }
    Ok(ids)
}

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

#[cfg(test)]
mod rule_tests {
    use super::*;

    fn point(system: ContactPointSystem, value: &str) -> ContactPoint {
        ContactPoint {
            system,
            value: value.into(),
            use_type: None,
        }
    }

    #[test]
    fn an_address_needs_something_to_route_by() {
        let line_only = Address {
            line1: Some("1 High St".into()),
            state: Some("X".into()),
            ..Address::default()
        };
        assert!(validate_address(&line_only).is_err());
        assert!(validate_address(&Address::default()).is_err());
        for a in [
            Address {
                city: Some("Leeds".into()),
                ..Address::default()
            },
            Address {
                postal_code: Some("LS1 1AA".into()),
                ..Address::default()
            },
            Address {
                country: Some("GB".into()),
                ..Address::default()
            },
        ] {
            assert!(validate_address(&a).is_ok());
        }
    }

    #[test]
    fn an_email_needs_an_at_and_a_dot() {
        let email = |v| validate_contact_point(&point(ContactPointSystem::Email, v));
        assert!(email("a@b.co").is_ok());
        assert!(email("a.b.co").is_err());
        assert!(email("a@bco").is_err());
    }

    #[test]
    fn a_phone_needs_seven_to_fifteen_digits() {
        for system in [
            ContactPointSystem::Phone,
            ContactPointSystem::Sms,
            ContactPointSystem::Fax,
        ] {
            let ok = |v: &str| validate_contact_point(&point(system, v)).is_ok();
            assert!(!ok("123456"));
            assert!(ok("1234567"));
            assert!(ok("+44 (0)20 7946 0958"));
            assert!(ok("123456789012345"));
            assert!(!ok("1234567890123456"));
        }
    }

    #[test]
    fn other_contact_systems_pass_unchecked() {
        let other = ContactPointSystem::Url;
        assert!(validate_contact_point(&point(other, "anything")).is_ok());
    }

    #[test]
    fn phone_numbers_keep_digits_and_a_leading_plus() {
        assert_eq!(normalize_phone(" +44 (0)20-7946 0958"), "+4402079460958");
        assert_eq!(normalize_phone("020 7946 0958"), "02079460958");
        assert_eq!(normalize_phone("a+1"), "1");
        assert_eq!(normalize_phone(""), "");
    }

    #[test]
    fn an_appointment_must_start_before_it_ends() {
        let t = crate::time::utc;
        let (a, b) = (t(2026, 11, 3, 9, 0, 0), t(2026, 11, 3, 9, 30, 0));
        let new = |s, e| Appointment::new(uuid::Uuid::nil(), s, e);
        assert!(validate_appointment(&new(a, b)).is_ok());
        assert!(validate_appointment(&new(a, a)).is_err(), "zero length");
        assert!(validate_appointment(&new(b, a)).is_err(), "inverted");
    }

    #[test]
    fn a_charge_cannot_be_negative() {
        use crate::models::{Iso4217, Money};
        let charge = |amount: &str| {
            Charge::new(
                uuid::Uuid::nil(),
                "X1".into(),
                "test".into(),
                Money {
                    amount: amount.parse().unwrap(),
                    currency: Iso4217::new("GBP").unwrap(),
                },
            )
        };
        assert!(validate_charge(&charge("0")).is_ok());
        assert!(validate_charge(&charge("12.50")).is_ok());
        assert!(validate_charge(&charge("-0.01")).is_err());
    }

    #[test]
    fn an_id_list_is_every_entry_an_id_or_an_error_naming_the_entry() {
        let (a, b) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        assert_eq!(uuid_list("ids", &format!("{a}, {b}")).unwrap(), vec![a, b]);
        assert_eq!(uuid_list("ids", &a.to_string()).unwrap(), vec![a]);
        for bad in ["", ",", "x", &format!("{a},x,{b}"), &format!("{a},")] {
            let e = uuid_list("ids", bad).unwrap_err().to_string();
            assert!(
                e.contains("ids") && e.contains("is not an id"),
                "{bad:?}: {e}"
            );
        }
        assert!(
            uuid_list("ids", &format!("{a},x"))
                .unwrap_err()
                .to_string()
                .contains("entry 2")
        );
        let many = vec![a.to_string(); MAX_ID_LIST + 1].join(",");
        assert!(
            uuid_list("ids", &many)
                .unwrap_err()
                .to_string()
                .contains("at most")
        );
        assert_eq!(
            uuid_list("ids", &vec![a.to_string(); MAX_ID_LIST].join(","))
                .unwrap()
                .len(),
            MAX_ID_LIST
        );
    }
}
