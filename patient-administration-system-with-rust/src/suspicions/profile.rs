//! Suspected-condition profiles (WPAS-8, spec 20, NFR-26): which conditions a deployment tracks, and for each
//! one its subtypes, its target, whether a patient's own cancel is held for a person, and its booking priority.
//!
//! No country is the core. A profile is a JSON document: one of the built-in files in
//! `profiles/suspected-conditions/` (`generic`, the default, and `wales`), or the path of an operator's own
//! file, named by `PAS_SUSPICION_PROFILE`. It is checked in full at start-up ([`crate::config::env::validate`]),
//! so a mistake stops the server rather than recording a wait against the wrong rule.
//!
//! What a profile cannot change is the clock: it starts at the point of suspicion and never pauses (FR-208).
//! The rules a suspicion was recorded under are copied onto it, so a later change of profile never rewrites a
//! wait that is already counting (FR-221).

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::models::waitlist::Priority;
use crate::{Error, Result};

/// The setting that names the profile.
pub const SETTING: &str = "PAS_SUSPICION_PROFILE";
/// The profile used when the setting is not set.
pub const DEFAULT: &str = "generic";

const BUILT_IN: [(&str, &str); 2] = [
    (
        "generic",
        include_str!("../../profiles/suspected-conditions/generic.json"),
    ),
    (
        "wales",
        include_str!("../../profiles/suspected-conditions/wales.json"),
    ),
];

/// The names of the built-in profiles.
pub fn built_in_names() -> Vec<&'static str> {
    BUILT_IN.iter().map(|(n, _)| *n).collect()
}

/// One subtype a condition may carry (a tumour site, a type of diabetes, a type of dementia).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Subtype {
    pub code: String,
    pub name: String,
}

/// A condition's list of subtypes. Its `scheme` names the list and is stored with each code, because the
/// same code means different things in different lists.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SubtypeList {
    pub scheme: String,
    pub title: String,
    /// Where the list comes from.
    pub source: String,
    pub codes: Vec<Subtype>,
    /// Whether a suspicion of this condition must name a subtype.
    #[serde(default)]
    pub required: bool,
}

/// A published target: days from the point of suspicion to the clock stopping.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub days: i64,
    /// The standard in words, as its publisher puts it.
    #[serde(default)]
    #[schema(required = true)]
    pub standard: Option<String>,
    /// Who publishes it. A target without a source is refused: the PAS does not invent targets (FR-211).
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// Stored on each suspicion: lower case, digits and `_`.
    pub code: String,
    pub name: String,
    #[serde(default)]
    #[schema(required = true)]
    pub subtypes: Option<SubtypeList>,
    #[serde(default)]
    #[schema(required = true)]
    pub target: Option<Target>,
    /// A patient's own cancel by reply, on an appointment from a referral with an open suspicion of this
    /// condition, is held for a person to phone first (FR-214).
    #[serde(default)]
    pub hold_patient_cancel: bool,
    /// The waiting-list priority a referral with an open suspicion of this condition is raised to, never
    /// lowered (FR-220).
    #[serde(default)]
    #[schema(required = true)]
    pub booking_priority: Option<Priority>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Stored on each suspicion.
    pub name: String,
    pub title: String,
    #[serde(default)]
    #[schema(required = true)]
    pub notes: Option<String>,
    pub conditions: Vec<Condition>,
}

fn is_code(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

impl Profile {
    /// Read and check a profile document.
    pub fn from_json(text: &str) -> Result<Self> {
        let p: Profile = serde_json::from_str(text)
            .map_err(|e| Error::validation(format!("{SETTING}: not a valid profile: {e}")))?;
        p.check()?;
        Ok(p)
    }

    /// A built-in profile by name, or an operator's file by path.
    pub fn load(name_or_path: &str) -> Result<Self> {
        let key = name_or_path.trim();
        if let Some((_, text)) = BUILT_IN.iter().find(|(n, _)| *n == key) {
            return Self::from_json(text);
        }
        if key.ends_with(".json") {
            let text = std::fs::read_to_string(key)
                .map_err(|e| Error::validation(format!("{SETTING}: cannot read {key}: {e}")))?;
            return Self::from_json(&text);
        }
        Err(Error::validation(format!(
            "{SETTING} must be one of {} or the path of a .json profile",
            built_in_names().join(", ")
        )))
    }

    /// The profile the setting names, or the default. The setting was checked at start-up.
    pub fn from_env() -> Self {
        let name = crate::config::env::text(SETTING).unwrap_or_else(|| DEFAULT.to_string());
        Self::load(&name).unwrap_or_else(|e| panic!("{e}"))
    }

    fn check(&self) -> Result<()> {
        let bad = |why: String| Err(Error::validation(format!("{SETTING} {}: {why}", self.name)));
        if !is_code(&self.name, 64) {
            return bad("the name must be lower case letters, digits, '-' or '_'".into());
        }
        if self.conditions.is_empty() {
            return bad("a profile lists at least one condition".into());
        }
        let mut codes = BTreeSet::new();
        for c in &self.conditions {
            if !is_code(&c.code, 64) || c.code.contains('-') {
                return bad(format!(
                    "condition code {:?} must be lower case letters, digits or '_'",
                    c.code
                ));
            }
            if !codes.insert(c.code.as_str()) {
                return bad(format!("condition {} is listed twice", c.code));
            }
            if c.name.trim().is_empty() {
                return bad(format!("condition {} has no name", c.code));
            }
            if let Some(t) = &c.target
                && (t.days <= 0 || t.days > 3650 || t.source.trim().is_empty())
            {
                return bad(format!(
                    "condition {}: a target is 1 to 3650 days and names its source",
                    c.code
                ));
            }
            if let Some(s) = &c.subtypes {
                if !is_code(&s.scheme, 64) || s.source.trim().is_empty() || s.codes.is_empty() {
                    return bad(format!(
                        "condition {}: a subtype list has a scheme name, a source and at least one code",
                        c.code
                    ));
                }
                let mut seen = BTreeSet::new();
                for st in &s.codes {
                    let shape = !st.code.is_empty()
                        && st.code.len() <= 16
                        && st
                            .code
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
                    if !shape || st.name.trim().is_empty() || !seen.insert(st.code.as_str()) {
                        return bad(format!(
                            "condition {}: subtype {:?} is badly formed, unnamed or listed twice",
                            c.code, st.code
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn condition(&self, code: &str) -> Option<&Condition> {
        self.conditions.iter().find(|c| c.code == code)
    }

    pub fn condition_codes(&self) -> Vec<&str> {
        self.conditions.iter().map(|c| c.code.as_str()).collect()
    }
}

impl Condition {
    pub fn target_days(&self) -> Option<i64> {
        self.target.as_ref().map(|t| t.days)
    }

    /// The name of a subtype in this condition's list, if it is one.
    pub fn subtype_name(&self, code: &str) -> Option<&str> {
        self.subtypes
            .as_ref()?
            .codes
            .iter()
            .find(|s| s.code == code)
            .map(|s| s.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_built_in_profile_loads_and_the_default_is_one() {
        for name in built_in_names() {
            let p = Profile::load(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(p.name, name, "a built-in profile's name is its file's");
        }
        assert!(built_in_names().contains(&DEFAULT));
    }

    #[test]
    fn test_the_generic_profile_sets_no_target() {
        let p = Profile::load("generic").unwrap();
        for c in &p.conditions {
            assert!(
                c.target.is_none(),
                "{}: the default profile invents no target",
                c.code
            );
        }
        assert!(p.condition("cancer").unwrap().hold_patient_cancel);
        assert!(
            p.condition("dementia")
                .unwrap()
                .subtype_name("alzheimers")
                .is_some()
        );
    }

    #[test]
    fn test_the_wales_profile_has_the_cancer_rules() {
        let p = Profile::load("wales").unwrap();
        let c = p.condition("cancer").unwrap();
        assert_eq!(c.target_days(), Some(62));
        assert_eq!(c.booking_priority, Some(Priority::Urgent));
        let sites = c.subtypes.as_ref().unwrap();
        assert!(sites.required);
        assert_eq!(
            sites
                .codes
                .iter()
                .map(|s| s.code.as_str())
                .collect::<Vec<_>>(),
            [
                "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14",
                "98"
            ]
        );
    }

    #[test]
    fn test_a_bad_profile_is_refused_with_a_reason() {
        let ok = r#"{"name":"x","title":"X","conditions":[{"code":"a","name":"A"}]}"#;
        assert!(Profile::from_json(ok).is_ok());
        for (bad, why) in [
            (
                r#"{"name":"x","title":"X","conditions":[]}"#,
                "no conditions",
            ),
            (
                r#"{"name":"X Y","title":"X","conditions":[{"code":"a","name":"A"}]}"#,
                "a bad name",
            ),
            (
                r#"{"name":"x","title":"X","conditions":[{"code":"a","name":"A"},{"code":"a","name":"B"}]}"#,
                "a condition twice",
            ),
            (
                r#"{"name":"x","title":"X","conditions":[{"code":"a","name":"A","target":{"days":62,"source":""}}]}"#,
                "a target with no source",
            ),
            (
                r#"{"name":"x","title":"X","conditions":[{"code":"a","name":"A","target":{"days":0,"source":"s"}}]}"#,
                "a target of no days",
            ),
            (
                r#"{"name":"x","title":"X","conditions":[{"code":"a","name":"A","subtypes":{"scheme":"s","title":"t","source":"s","codes":[{"code":"1","name":"One"},{"code":"1","name":"Again"}]}}]}"#,
                "a subtype twice",
            ),
            (
                r#"{"name":"x","title":"X","conditions":[{"code":"a","name":"A","booking_priority":"soonish"}]}"#,
                "an unknown priority",
            ),
            (
                r#"{"name":"x","title":"X","conditions":[{"code":"a","name":"A","colour":"red"}]}"#,
                "an unknown field",
            ),
        ] {
            assert!(Profile::from_json(bad).is_err(), "{why}");
        }
        assert!(Profile::load("atlantis").is_err(), "an unknown name");
        assert!(
            Profile::load("/no/such/profile.json").is_err(),
            "a missing file"
        );
    }
}
