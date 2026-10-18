//! OpenAPI spec aggregator — **stubbed in v0.44.0 Phase 2**.
//!
//! The Phase 2 stub doc lists only `/api/health` and the FHIR bundle
//! placeholder request/response shapes. Every Patient / Practitioner /
//! HL7 v2 handler annotation in v0.43.0 is removed pending Phase 3.
//!
//! Phase 3 reintroduces the full spec aggregator over the
//! proxy-rewired handlers in `rest::handlers` + `fhir::handlers`.

use utoipa::OpenApi;

use crate::api::fhir::operation_outcome::{Issue, OperationOutcome};
use crate::api::fhir::resources::{
    FhirBundleRequest, FhirBundleResponse, FhirBundleWriteEntry, FhirWriteBundle,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Patient Administration System",
        description = "Phase 2 stub OpenAPI doc. Most v0.43.0 routes return 503 + UPSTREAM_UNAVAILABLE for the duration of the v0.44.0 Option C identity-delegation migration. Phase 3 reintroduces the full annotated handler surface.",
        version = "0.44.0-phase2"
    ),
    tags(
        (name = "Health", description = "Health + readiness checks.")
    ),
    components(schemas(
        OperationOutcome,
        Issue,
        FhirBundleRequest,
        FhirBundleResponse,
        FhirBundleWriteEntry,
        FhirWriteBundle,
    ))
)]
pub struct ApiDoc;
