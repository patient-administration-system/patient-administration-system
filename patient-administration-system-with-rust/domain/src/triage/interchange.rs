//! A triage category as a FHIR `Observation` (NEW-24).
//!
//! The category is coded with the SNOMED CT UK "emergency care acuity" concepts that the NHS Wales Data
//! Dictionary lists for *Urgent and Emergency Care Acuity* (valid from 1 August 2024), each mapped there to
//! the Manchester Triage System level. The PAS's own category sits beside it as a second coding, so the
//! value survives a consumer that ignores SNOMED. The Welsh EDDS *Triage Category* (01 to 05) is available
//! as [`wales_triage_category`]. **Not verified:** the same concepts in the England ECDS acuity set (no
//! document was found that says so), a SNOMED concept for the Observation's own `code` (so it carries text
//! only), and any HL7 v2 field for the category (see `spec/09-api-surface.md`).

use serde_json::{Value, json};

use super::{CATEGORIES, TriageAssessment};

/// The SNOMED CT system URI.
pub const SNOMED: &str = "http://snomed.info/sct";
/// The PAS's own code system for the five categories.
pub const CATEGORY_SYSTEM: &str = "urn:patient-administration-system:triage-category";

/// SNOMED CT UK concept id and preferred term for each category, in the order of [`CATEGORIES`].
const ACUITY: [(&str, &str); 5] = [
    (
        "1064891000000107",
        "Immediate resuscitation level emergency care",
    ),
    ("1064911000000105", "Very urgent level emergency care"),
    ("1064901000000108", "Urgent level emergency care"),
    ("1077241000000103", "Standard level emergency care"),
    ("1077251000000100", "Non-urgent level emergency care"),
];

fn index(category: &str) -> Option<usize> {
    CATEGORIES.iter().position(|c| *c == category)
}

/// The SNOMED CT concept id for a category.
pub fn snomed_code(category: &str) -> Option<&'static str> {
    index(category).map(|i| ACUITY[i].0)
}

/// The category for a SNOMED CT concept id from the acuity set.
pub fn category_of_snomed(code: &str) -> Option<&'static str> {
    ACUITY
        .iter()
        .position(|(c, _)| *c == code)
        .map(|i| CATEGORIES[i])
}

/// The NHS Wales EDDS *Triage Category* code (`01` to `05`) for a category.
pub fn wales_triage_category(category: &str) -> Option<&'static str> {
    index(category).map(|i| ["01", "02", "03", "04", "05"][i])
}

/// An assessment as a FHIR R5 `Observation` for the patient `person_id`.
pub fn to_observation(a: &TriageAssessment, person_id: uuid::Uuid) -> Value {
    let mut codings = Vec::new();
    let mut text = a.category.clone();
    if let Some(i) = index(&a.category) {
        codings.push(json!({ "system": SNOMED, "code": ACUITY[i].0, "display": ACUITY[i].1 }));
        text = ACUITY[i].1.to_string();
    }
    codings.push(json!({ "system": CATEGORY_SYSTEM, "code": a.category }));
    let mut o = json!({
        "resourceType": "Observation",
        "id": a.id,
        "status": "final",
        "category": [{ "coding": [{
            "system": "http://terminology.hl7.org/CodeSystem/observation-category",
            "code": "survey",
        }] }],
        "code": { "text": "Urgent and Emergency Care Acuity" },
        "subject": { "reference": format!("Patient/{person_id}") },
        "encounter": { "reference": format!("Encounter/{}", a.encounter_id) },
        "effectiveDateTime": a.assessed_at,
        "performer": [{ "display": a.assessed_by }],
        "valueCodeableConcept": { "coding": codings, "text": text },
    });
    let method = match &a.system_version {
        Some(v) => format!("Manchester Triage System {v}"),
        None => "Manchester Triage System".to_string(),
    };
    o["method"] = json!({ "text": method });
    if let Some(f) = &a.flowchart {
        o["note"] = json!([{ "text": format!("Flowchart: {f}") }]);
    }
    o
}

/// The category carried by an `Observation` made by [`to_observation`] (or by anyone using the SNOMED acuity
/// concepts or the PAS code system).
pub fn category_of_observation(o: &Value) -> Option<&'static str> {
    o["valueCodeableConcept"]["coding"]
        .as_array()?
        .iter()
        .find_map(|c| {
            let code = c["code"].as_str()?;
            match c["system"].as_str()? {
                SNOMED => category_of_snomed(code),
                CATEGORY_SYSTEM => CATEGORIES.iter().find(|k| **k == code).copied(),
                _ => None,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn assessment(category: &str) -> TriageAssessment {
        TriageAssessment {
            id: Uuid::new_v4(),
            encounter_id: Uuid::new_v4(),
            triage_system: "mts".into(),
            system_version: Some("3.8".into()),
            category: category.into(),
            colour: super::super::colour(category),
            flowchart: Some("Chest pain".into()),
            assessed_by: "Nurse Okafor".into(),
            assessed_at: Utc::now(),
        }
    }

    #[test]
    fn every_category_round_trips_through_an_observation_and_through_snomed() {
        for c in CATEGORIES {
            let a = assessment(c);
            let o = to_observation(&a, Uuid::new_v4());
            assert_eq!(category_of_observation(&o), Some(c), "{c}");
            assert_eq!(category_of_snomed(snomed_code(c).unwrap()), Some(c));
        }
    }

    #[test]
    fn the_observation_names_patient_encounter_assessor_and_method() {
        let p = Uuid::new_v4();
        let a = assessment("very_urgent");
        let o = to_observation(&a, p);
        assert_eq!(o["resourceType"], "Observation");
        assert_eq!(o["status"], "final");
        assert_eq!(o["subject"]["reference"], format!("Patient/{p}"));
        assert_eq!(
            o["encounter"]["reference"],
            format!("Encounter/{}", a.encounter_id)
        );
        assert_eq!(o["performer"][0]["display"], "Nurse Okafor");
        assert_eq!(o["method"]["text"], "Manchester Triage System 3.8");
        assert_eq!(
            o["valueCodeableConcept"]["coding"][0]["code"],
            "1064911000000105"
        );
        assert_eq!(o["note"][0]["text"], "Flowchart: Chest pain");
    }

    #[test]
    fn the_codes_are_distinct_and_the_welsh_category_runs_01_to_05() {
        let mut codes: Vec<_> = CATEGORIES.iter().map(|c| snomed_code(c).unwrap()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 5);
        let welsh: Vec<_> = CATEGORIES
            .iter()
            .map(|c| wales_triage_category(c).unwrap())
            .collect();
        assert_eq!(welsh, ["01", "02", "03", "04", "05"]);
        assert_eq!(snomed_code("red"), None);
        assert_eq!(category_of_snomed("1"), None);
    }

    #[test]
    fn a_foreign_or_missing_value_gives_no_category() {
        assert_eq!(category_of_observation(&json!({})), None);
        let o =
            json!({"valueCodeableConcept": {"coding": [{"system": "urn:x", "code": "urgent"}]}});
        assert_eq!(category_of_observation(&o), None);
    }
}
