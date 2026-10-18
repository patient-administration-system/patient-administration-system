//! FHIR R5 resource types — **stubbed in v0.44.0 Phase 2**.
//!
//! The full FhirPatient / FhirPractitioner / FhirEncounter / FhirAppointment
//! / FhirCoverage / FhirSchedule / FhirSlot / FhirLocation / FhirBundle
//! converter surface that lived here through v0.43.0 is removed pending
//! Phase 3 rewrite. Phase 3 lands typed `FhirPerson` / `FhirWorker`
//! shapes that proxy through `crate::upstream` and keeps the
//! PAS-owned-resource converters (Encounter / Appointment / Coverage /
//! Schedule / Slot / Location) intact against the renamed `person_id` /
//! `worker_id` columns.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// FHIR Bundle request body — Phase 2 placeholder.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct FhirBundleRequest {
    #[serde(rename = "resourceType", default = "default_bundle_resource_type")]
    pub resource_type: String,
    #[serde(rename = "type")]
    pub bundle_type: String,
    #[serde(default)]
    pub entry: serde_json::Value,
}

fn default_bundle_resource_type() -> String {
    "Bundle".to_string()
}

/// FHIR Bundle response — Phase 2 placeholder.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct FhirBundleResponse {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "type")]
    pub bundle_type: String,
    pub entry: Vec<serde_json::Value>,
}

/// Per-entry write outcome — Phase 2 placeholder.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct FhirBundleWriteEntry {
    pub response: serde_json::Value,
}

/// Wrapper for incoming write Bundles — Phase 2 placeholder.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct FhirWriteBundle {
    #[serde(flatten)]
    pub raw: serde_json::Value,
}
