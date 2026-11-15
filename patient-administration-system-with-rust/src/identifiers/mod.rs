//! Identifier schemes (QUAL-40 finding 1, NFR-26): the checks the PAS knows for a person's identifiers.
//!
//! No country is the core. A scheme is matched by the system URI an identifier declares, so a check runs only
//! on an identifier that says it belongs to that scheme, and an identifier of any other system passes through
//! untouched to person-service, which owns the person record. Each national scheme is an adapter in a file of
//! its own; adding one is a new file and a line in [`SCHEMES`], with no change to the request path.
//!
//! A check says a value is well formed. It never says the value was issued or belongs to this person: that
//! needs the issuer's own lookup, which the PAS does not do.

use serde::Serialize;

use crate::{Error, Result};

pub mod gb_nhs_number;

/// One identifier scheme.
#[derive(Debug, Clone, Copy)]
pub struct Scheme {
    /// A stable code for the scheme: the ISO 3166-1 country code and the scheme's own name.
    pub code: &'static str,
    /// The identifier system URI the scheme's values carry, as in FHIR `Identifier.system`.
    pub system: &'static str,
    /// The scheme's name, as its issuer writes it.
    pub name: &'static str,
    /// Check a value; returns it normalised (separators removed).
    pub check: fn(&str) -> Result<String>,
    /// Write a normalised value the usual way for the scheme.
    pub format: fn(&str) -> String,
}

/// Every scheme the PAS can check.
pub const SCHEMES: &[Scheme] = &[gb_nhs_number::SCHEME];

/// The scheme for a system URI, if the PAS has one.
pub fn by_system(system: &str) -> Option<&'static Scheme> {
    SCHEMES.iter().find(|s| s.system == system)
}

/// What a client sees of a scheme.
#[derive(Debug, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct IdentifierScheme {
    pub code: String,
    pub system: String,
    pub name: String,
}

impl From<&Scheme> for IdentifierScheme {
    fn from(s: &Scheme) -> Self {
        Self {
            code: s.code.to_string(),
            system: s.system.to_string(),
            name: s.name.to_string(),
        }
    }
}

/// A value checked against its scheme.
#[derive(Debug, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct IdentifierChecked {
    pub system: String,
    pub valid: bool,
    /// The value with separators removed.
    pub value: String,
    /// The value written the usual way for its scheme.
    pub formatted: String,
}

/// Check one value against the scheme for `system`. A system with no scheme is refused: the caller asked
/// for a check the PAS cannot make, and saying "valid" would be a guess.
pub fn check(system: &str, value: &str) -> Result<IdentifierChecked> {
    let scheme = by_system(system).ok_or_else(|| {
        Error::validation(
            "no check is known for that identifier system; see GET /api/identifier-schemes",
        )
    })?;
    let value = (scheme.check)(value)?;
    Ok(IdentifierChecked {
        system: scheme.system.to_string(),
        valid: true,
        formatted: (scheme.format)(&value),
        value,
    })
}

/// Refuse a person body whose `identifiers` carry a value that fails its scheme's check. Identifiers of a
/// system with no scheme, and bodies with none, pass untouched.
pub fn check_person_identifiers(fields: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    let Some(ids) = fields.get("identifiers").and_then(|v| v.as_array()) else {
        return Ok(());
    };
    for id in ids {
        let Some(scheme) = id
            .get("system")
            .and_then(|v| v.as_str())
            .and_then(by_system)
        else {
            continue;
        };
        let value = id.get("value").and_then(|v| v.as_str()).unwrap_or("");
        (scheme.check)(value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scheme_has_a_distinct_code_and_system() {
        let mut codes = std::collections::BTreeSet::new();
        let mut systems = std::collections::BTreeSet::new();
        for s in SCHEMES {
            assert!(codes.insert(s.code), "{} twice", s.code);
            assert!(systems.insert(s.system), "{} twice", s.system);
            assert!(s.system.starts_with("https://") || s.system.starts_with("urn:"));
            assert!(!s.name.trim().is_empty());
        }
    }

    #[test]
    fn an_unknown_system_is_refused_by_check_and_passed_by_a_person_body() {
        assert!(check("https://example.test/mrn", "A-1").is_err());
        let other = serde_json::json!({ "identifiers": [{ "system": "https://example.test/mrn", "value": "anything" }] });
        assert!(check_person_identifiers(other.as_object().unwrap()).is_ok());
        assert!(check_person_identifiers(&serde_json::Map::new()).is_ok());
    }

    #[test]
    fn a_known_system_is_checked_in_a_person_body() {
        let system = gb_nhs_number::SCHEME.system;
        let bad =
            serde_json::json!({ "identifiers": [{ "system": system, "value": "9434765918" }] });
        assert!(check_person_identifiers(bad.as_object().unwrap()).is_err());
        let good =
            serde_json::json!({ "identifiers": [{ "system": system, "value": "943 476 5919" }] });
        assert!(check_person_identifiers(good.as_object().unwrap()).is_ok());
        let c = check(system, "943-476-5919").unwrap();
        assert_eq!(
            (c.value.as_str(), c.formatted.as_str()),
            ("9434765919", "943 476 5919")
        );
    }
}
