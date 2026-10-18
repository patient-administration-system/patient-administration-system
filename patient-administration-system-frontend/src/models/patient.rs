//! Patient view-models — sourced from upstream `person-service` over HTTP.
//!
//! v0.1 read these straight from the local `patients` table. v0.2 (paired
//! with PAS v0.44.0 Option C identity delegation) drops that table and
//! moves identity to `person-service`. These helpers now wrap
//! [`crate::services::person_api`] and translate the upstream `Person`
//! shape into the [`PatientCard`] / [`PatientDetail`] view-models the
//! templates already render — so the controllers and templates stay
//! unchanged.

use serde::Serialize;
use uuid::Uuid;

use crate::services::person_api::{self, PersonApiError, UpstreamPerson};

#[derive(Serialize, Debug)]
pub struct PatientCard {
    pub id: Uuid,
    pub family: String,
    pub given: String,
    pub gender: String,
    pub birth_date: Option<String>,
    pub active: bool,
}

#[derive(Serialize, Debug)]
pub struct PatientDetail {
    pub id: Uuid,
    pub family: String,
    pub given: String,
    pub gender: String,
    pub birth_date: Option<String>,
    pub active: bool,
    pub deceased: bool,
    pub marital_status: Option<String>,
    pub mrn: Option<String>,
    pub primary_phone: Option<String>,
    pub primary_email: Option<String>,
    pub primary_address: Option<String>,
}

/// List active patients (capped at `limit`) from `person-service`.
/// Returns an empty list on transport / status / parse failure so the
/// dashboard / picker pages can still render a useful skeleton when
/// the upstream is briefly unavailable.
pub async fn list_active_patients(limit: u32) -> Vec<PatientCard> {
    match person_api::list_persons(limit).await {
        Ok(persons) => persons.into_iter().map(person_to_card).collect(),
        Err(e) => {
            eprintln!("list_active_patients: person-service unavailable ({e}); returning empty");
            Vec::new()
        }
    }
}

/// Fetch one patient by id. Maps:
/// - `Ok(Some(_))` → `Ok(Some(detail))`
/// - `Ok(None)` (upstream 404) → `Ok(None)`
/// - transport / status error → `Err`
pub async fn find_patient_by_id(id: Uuid) -> Result<Option<PatientDetail>, PersonApiError> {
    Ok(person_api::get_person(id).await?.map(person_to_detail))
}

fn person_to_card(p: UpstreamPerson) -> PatientCard {
    let (family, given) = name_parts(p.name.as_ref().or(p.names.as_ref()));
    PatientCard {
        id: p.id,
        family,
        given,
        gender: p.gender.unwrap_or_default(),
        birth_date: p.birth_date,
        active: p.active.unwrap_or(true),
    }
}

fn person_to_detail(p: UpstreamPerson) -> PatientDetail {
    let (family, given) = name_parts(p.name.as_ref().or(p.names.as_ref()));
    let mrn = first_identifier_value(p.identifiers.as_ref());
    let primary_phone = first_telecom_value(p.telecom.as_ref(), "phone");
    let primary_email = first_telecom_value(p.telecom.as_ref(), "email");
    let primary_address = first_address_line(p.addresses.as_ref());
    PatientDetail {
        id: p.id,
        family,
        given,
        gender: p.gender.unwrap_or_default(),
        birth_date: p.birth_date,
        active: p.active.unwrap_or(true),
        deceased: p.deceased.unwrap_or(false),
        marital_status: p.marital_status,
        mrn,
        primary_phone,
        primary_email,
        primary_address,
    }
}

fn name_parts(v: Option<&serde_json::Value>) -> (String, String) {
    let Some(v) = v else {
        return (String::new(), String::new());
    };
    let obj = match v {
        serde_json::Value::Array(a) => a.first(),
        serde_json::Value::Object(_) => Some(v),
        _ => None,
    };
    let Some(obj) = obj else {
        return (String::new(), String::new());
    };
    let family = obj
        .get("family")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let given = obj
        .get("given")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    (family, given)
}

fn first_identifier_value(v: Option<&serde_json::Value>) -> Option<String> {
    v?.as_array()?
        .first()
        .and_then(|i| i.get("value")?.as_str().map(String::from))
}

fn first_telecom_value(v: Option<&serde_json::Value>, system: &str) -> Option<String> {
    v?.as_array()?.iter().find_map(|c| {
        if c.get("system")?.as_str()? == system {
            c.get("value")?.as_str().map(String::from)
        } else {
            None
        }
    })
}

fn first_address_line(v: Option<&serde_json::Value>) -> Option<String> {
    let addr = v?.as_array()?.first()?;
    let line = addr
        .get("line")
        .and_then(|x| x.as_array())
        .and_then(|arr| arr.first())
        .and_then(|x| x.as_str())
        .or_else(|| addr.get("line1").and_then(|x| x.as_str()))?;
    let city = addr
        .get("city")
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    if city.is_empty() {
        Some(line.to_string())
    } else {
        Some(format!("{line}, {city}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_person(extra: serde_json::Value) -> UpstreamPerson {
        let mut v = json!({ "id": Uuid::new_v4().to_string() });
        if let (serde_json::Value::Object(m), serde_json::Value::Object(extra_m)) = (&mut v, extra)
        {
            for (k, val) in extra_m {
                m.insert(k, val);
            }
        }
        serde_json::from_value(v).expect("parse")
    }

    #[test]
    fn person_to_card_extracts_family_and_given() {
        let p = make_person(json!({
            "name": { "family": "Doe", "given": ["Jane", "M"] },
            "gender": "female",
            "birth_date": "1990-01-15"
        }));
        let card = person_to_card(p);
        assert_eq!(card.family, "Doe");
        assert_eq!(card.given, "Jane M");
        assert_eq!(card.gender, "female");
        assert_eq!(card.birth_date.as_deref(), Some("1990-01-15"));
        assert!(card.active);
    }

    #[test]
    fn person_to_detail_finds_mrn_and_phone() {
        let p = make_person(json!({
            "name": { "family": "Doe", "given": ["Jane"] },
            "identifiers": [{ "value": "MRN-99" }],
            "telecom": [{ "system": "phone", "value": "(555)555-0100" }]
        }));
        let d = person_to_detail(p);
        assert_eq!(d.mrn.as_deref(), Some("MRN-99"));
        assert_eq!(d.primary_phone.as_deref(), Some("(555)555-0100"));
    }

    #[test]
    fn person_to_detail_address_handles_fhir_line_array() {
        let p = make_person(json!({
            "addresses": [{ "line": ["123 Elm"], "city": "Springfield" }]
        }));
        let d = person_to_detail(p);
        assert_eq!(d.primary_address.as_deref(), Some("123 Elm, Springfield"));
    }

    #[test]
    fn person_to_detail_address_handles_legacy_line1() {
        let p = make_person(json!({
            "addresses": [{ "line1": "Old Field" }]
        }));
        let d = person_to_detail(p);
        assert_eq!(d.primary_address.as_deref(), Some("Old Field"));
    }

    #[test]
    fn empty_name_yields_empty_strings() {
        let p = make_person(json!({}));
        let card = person_to_card(p);
        assert_eq!(card.family, "");
        assert_eq!(card.given, "");
    }
}
