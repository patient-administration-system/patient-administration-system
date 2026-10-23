//! What the FHIR endpoint can do, said once (NEW-21).
//!
//! [`SUPPORTED`] is the list of resource types and interactions that work. `/fhir/metadata` publishes it as
//! a CapabilityStatement, and `tests/new21_capability_test.rs` sends every listed interaction to the router
//! and fails if one answers 503, 405 or the router's own 404, so the statement cannot claim more than the
//! server does. A new FHIR route goes here as well.

use serde_json::{Value, json};

/// One resource type and the FHIR interactions the server supports for it.
pub struct Supported {
    pub resource: &'static str,
    pub interactions: &'static [&'static str],
}

/// Every resource type and interaction the FHIR endpoint supports.
pub const SUPPORTED: &[Supported] = &[
    Supported {
        resource: "Patient",
        interactions: &["create", "read", "update", "delete", "search-type"],
    },
    Supported {
        resource: "Practitioner",
        interactions: &["create", "read", "update", "delete"],
    },
    Supported {
        resource: "Schedule",
        interactions: &["create", "read", "update", "delete"],
    },
    Supported {
        resource: "Slot",
        interactions: &["create", "read", "update", "delete"],
    },
    Supported {
        resource: "Appointment",
        interactions: &["read"],
    },
    Supported {
        resource: "Encounter",
        interactions: &["read"],
    },
    Supported {
        resource: "Location",
        interactions: &["read"],
    },
    Supported {
        resource: "Coverage",
        interactions: &["read"],
    },
    Supported {
        resource: "Flag",
        interactions: &["search-type"],
    },
    Supported {
        resource: "Observation",
        interactions: &["search-type"],
    },
    Supported {
        resource: "SubscriptionTopic",
        interactions: &["search-type"],
    },
    Supported {
        resource: "Subscription",
        interactions: &["search-type"],
    },
];

/// The FHIR R5 CapabilityStatement for this server, built from [`SUPPORTED`].
pub fn statement() -> Value {
    let resources: Vec<Value> = SUPPORTED
        .iter()
        .map(|s| {
            json!({
                "type": s.resource,
                "interaction": s.interactions.iter().map(|code| json!({ "code": code })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "resourceType": "CapabilityStatement",
        "status": "active",
        // When SUPPORTED last changed; FHIR asks for the date the statement was last revised.
        "date": "2026-10-23",
        "kind": "instance",
        "software": { "name": "patient-administration-system", "version": env!("CARGO_PKG_VERSION") },
        "implementation": {
            "description": "Patient Administration System FHIR endpoint. Patients and practitioners are owned by \
                            person-service and worker-service and reached through them; appointments and \
                            encounters are written through the typed REST API (POST /api/slots/{id}/book, \
                            POST /api/admissions)."
        },
        "fhirVersion": "5.0.0",
        "format": ["json"],
        "rest": [{ "mode": "server", "resource": resources }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_statement_lists_exactly_the_supported_interactions() {
        let s = statement();
        assert_eq!(s["resourceType"], "CapabilityStatement");
        assert_eq!(s["fhirVersion"], "5.0.0");
        let resources = s["rest"][0]["resource"].as_array().unwrap();
        assert_eq!(resources.len(), SUPPORTED.len());
        for (r, want) in resources.iter().zip(SUPPORTED) {
            assert_eq!(r["type"], want.resource);
            let codes: Vec<&str> = r["interaction"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["code"].as_str().unwrap())
                .collect();
            assert_eq!(codes, want.interactions);
        }
    }

    #[test]
    fn every_interaction_is_a_fhir_code() {
        const CODES: [&str; 10] = [
            "read",
            "vread",
            "update",
            "patch",
            "delete",
            "history-instance",
            "history-type",
            "create",
            "search-type",
            "conditional-create",
        ];
        for s in SUPPORTED {
            for i in s.interactions {
                assert!(CODES.contains(i), "{}: {i}", s.resource);
            }
        }
    }
}
