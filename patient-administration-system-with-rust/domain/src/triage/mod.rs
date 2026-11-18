//! Triage (MTS categories): the categories and their colours, the assessment record, the interchange codes, the
//! targets and the waits arithmetic. Recording assessments and reading the queue need the database and live in
//! the server's `triage` module, which re-exports this one.

pub mod interchange;
pub mod targets;
pub mod waits;

use jiff::Timestamp;
use serde::Serialize;
use uuid::Uuid;

use crate::{Error, Result};

/// The triage systems whose result can be recorded.
pub const SYSTEMS: [&str; 1] = ["mts"];

/// The five categories, most urgent first. The colour is a display label only.
pub const CATEGORIES: [&str; 5] = [
    "immediate",
    "very_urgent",
    "urgent",
    "standard",
    "non_urgent",
];

/// The display colour for a category.
pub fn colour(category: &str) -> Option<&'static str> {
    match category {
        "immediate" => Some("red"),
        "very_urgent" => Some("orange"),
        "urgent" => Some("yellow"),
        "standard" => Some("green"),
        "non_urgent" => Some("blue"),
        _ => None,
    }
}

pub const MAX_TEXT: usize = 100;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TriageAssessment {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub triage_system: String,
    #[schema(required = true)]
    pub system_version: Option<String>,
    pub category: String,
    #[schema(required = true)]
    pub colour: Option<&'static str>,
    #[schema(required = true)]
    pub flowchart: Option<String>,
    pub assessed_by: String,
    pub assessed_at: Timestamp,
}

/// Optional free text: trimmed, blank becomes none, bounded, no control characters.
pub fn optional_text(value: Option<&str>, field: &str) -> Result<Option<String>> {
    let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if v.chars().count() > MAX_TEXT || v.chars().any(char::is_control) {
        return Err(Error::validation(format!(
            "{field} must be at most {MAX_TEXT} characters with no control characters"
        )));
    }
    Ok(Some(v.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_has_a_colour_and_unknown_ones_none() {
        for c in CATEGORIES {
            assert!(colour(c).is_some(), "{c}");
        }
        assert_eq!(colour("red"), None);
    }

    #[test]
    fn optional_text_is_trimmed_bounded_and_clean() {
        assert_eq!(
            optional_text(Some("  3.8 "), "v").unwrap().as_deref(),
            Some("3.8")
        );
        assert_eq!(optional_text(Some("  "), "v").unwrap(), None);
        assert_eq!(optional_text(None, "v").unwrap(), None);
        assert!(optional_text(Some(&"x".repeat(101)), "v").is_err());
        assert!(optional_text(Some("a\nb"), "v").is_err());
    }
}
