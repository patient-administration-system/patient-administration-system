//! FHIR `OperationOutcome` error envelope, built with the `fhir` crate's typed R5 resource.
//!
//! Used by FHIR handlers to report errors in a way clinical clients expect: one issue with a severity, a
//! code and a free-text `diagnostics`. The value is a [`fhir::r5::resources::Resource`], so it serialises with its
//! `resourceType` like any other resource.

use fhir::r5::resources::Resource;
use serde_json::json;

/// What a handler returns for an error: an `OperationOutcome` resource.
pub type OperationOutcome = Resource;

/// Build an error-severity `OperationOutcome` with one issue.
pub fn error(code: &str, diagnostics: impl Into<String>) -> OperationOutcome {
    outcome(code, Some(diagnostics.into()))
}

/// Convenience for `code = "not-found"` errors.
pub fn not_found(diagnostics: impl Into<String>) -> OperationOutcome {
    error("not-found", diagnostics)
}

/// Convenience for `code = "exception"` errors.
pub fn exception(diagnostics: impl Into<String>) -> OperationOutcome {
    error("exception", diagnostics)
}

fn outcome(code: &str, diagnostics: Option<String>) -> OperationOutcome {
    let mut issue = json!({ "severity": "error", "code": code });
    if let Some(d) = diagnostics {
        issue["diagnostics"] = json!(d);
    }
    let shape = json!({ "resourceType": "OperationOutcome", "issue": [issue] });
    // Codes outside the FHIR value set are kept as written, so this shape always reads back; the fallback is
    // the same shape with a code that is certainly in the set.
    serde_json::from_value(shape).unwrap_or_else(|_| {
        serde_json::from_value(json!({
            "resourceType": "OperationOutcome",
            "issue": [{ "severity": "error", "code": "exception" }]
        }))
        .unwrap_or_else(|_| unreachable!("a constant OperationOutcome reads as a Resource"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_outcome_error_shape() {
        let v = serde_json::to_value(error("invalid", "bad UUID")).expect("serialize");
        assert_eq!(v["resourceType"], "OperationOutcome");
        assert_eq!(v["issue"].as_array().map(Vec::len), Some(1));
        assert_eq!(v["issue"][0]["severity"], "error");
        assert_eq!(v["issue"][0]["code"], "invalid");
        assert_eq!(v["issue"][0]["diagnostics"], "bad UUID");
    }

    #[test]
    fn test_operation_outcome_serializes_resource_type() {
        let json = serde_json::to_string(&not_found("patient missing")).expect("serialize");
        assert!(json.contains("\"resourceType\":\"OperationOutcome\""));
        assert!(json.contains("\"code\":\"not-found\""));
        assert!(json.contains("\"diagnostics\":\"patient missing\""));
    }

    #[test]
    fn test_a_code_outside_the_value_set_is_kept() {
        let v = serde_json::to_value(error("made-up", "x")).expect("serialize");
        assert_eq!(v["issue"][0]["code"], "made-up");
    }
}
