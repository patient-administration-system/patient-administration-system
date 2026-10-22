//! Part of [`super`]: bundle.

use super::*;

/// The server's CapabilityStatement: which resources and interactions it supports (NEW-21).
#[utoipa::path(
    get,
    path = "/fhir/metadata",
    tag = "FHIR",
    summary = "CapabilityStatement",
    responses((status = 200, description = "The FHIR R5 CapabilityStatement")),
)]
pub async fn metadata() -> Json<Value> {
    Json(crate::api::fhir::capability::statement())
}

/// Bundle handler. Walks `entry[*].resource.resourceType` and
/// enforces Decision #4: any entry of type `Patient` or `Practitioner`
/// is rejected with `invalid` so callers learn the correct routing
/// (those resources are owned by person-service / worker-service and
/// must be posted directly there, never proxied through a PAS Bundle).
/// Other entries return `not-supported` for now — typed FHIR creators
/// for non-Patient/Practitioner resources are wired one-at-a-time.
#[utoipa::path(
    post,
    path = "/fhir",
    tag = "FHIR",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn post_bundle(State(_state): State<AppState>, Json(body): Json<Value>) -> FhirResult {
    let entries = body
        .get("entry")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(operation_outcome::error(
                "invalid",
                "Bundle.entry is required and must be non-empty",
            )),
        ));
    }
    let mut response_entries = Vec::with_capacity(entries.len());
    for (idx, entry) in entries.iter().enumerate() {
        let rt = entry
            .get("resource")
            .and_then(|r| r.get("resourceType"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (status, detail) = match rt {
            "" => ("400", "entry.resource.resourceType is required"),
            "Patient" => (
                "400",
                "Patient entries are owned by person-service; POST to /api/persons directly",
            ),
            "Practitioner" => (
                "400",
                "Practitioner entries are owned by worker-service; POST to /api/practitioners directly",
            ),
            _ => (
                "501",
                "typed FHIR creator for this resource is not yet wired",
            ),
        };
        response_entries.push(serde_json::json!({
            "response": {
                "status": status,
                "outcome": {
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": if status == "400" { "invalid" } else { "not-supported" },
                        "diagnostics": format!("entry[{idx}] resourceType={rt:?}: {detail}"),
                    }]
                }
            }
        }));
    }
    let any_rejected = response_entries.iter().any(|e| {
        e.get("response")
            .and_then(|r| r.get("status"))
            .and_then(|s| s.as_str())
            .map(|s| s != "201" && s != "200")
            .unwrap_or(true)
    });
    let http_status = if any_rejected {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    let bundle_response = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction-response",
        "entry": response_entries,
    });
    // FhirResult is Result<Json<Value>, ...>; on a partial failure we
    // still want the structured body, so wrap into the error variant
    // with the appropriate HTTP code.
    if any_rejected {
        Err((
            http_status,
            Json(operation_outcome::error(
                "invalid",
                "Bundle contained entries that cannot be processed; see entry.response.outcome for details",
            )),
        ))
    } else {
        Ok(Json(bundle_response))
    }
}
