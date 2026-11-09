//! REST handler for patient enquiry (WPAS-3, FR-217).

use axa::extract::Path;
use axum::extract::State;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_json};

/// `GET /api/patients/{id}/enquiry`: what the PAS holds for a person, from one route. Read-audited as one
/// access (NEW-9).
#[utoipa::path(
    get,
    path = "/api/patients/{id}/enquiry",
    tag = "Patients",
    params(("id" = String, Path, description = "The person (a person-service id)")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::enquiry::Enquiry>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn enquiry(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match crate::enquiry::enquiry(&state.db, &state.referrals, &state.suspicions, id).await {
        Ok(e) => ok_json(e),
        Err(e) => error_to_response(e),
    }
}
