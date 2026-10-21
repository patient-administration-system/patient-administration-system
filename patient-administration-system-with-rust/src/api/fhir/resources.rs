//! FHIR R5 request and response shapes for Bundles. The resources themselves are built from PAS rows or
//! forwarded from `person-service` / `worker-service` in `handlers.rs`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// FHIR Bundle request body.
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

/// FHIR Bundle response.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct FhirBundleResponse {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "type")]
    pub bundle_type: String,
    pub entry: Vec<serde_json::Value>,
}

/// Per-entry write outcome.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct FhirBundleWriteEntry {
    pub response: serde_json::Value,
}

/// Wrapper for incoming write Bundles.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct FhirWriteBundle {
    #[serde(flatten)]
    pub raw: serde_json::Value,
}
