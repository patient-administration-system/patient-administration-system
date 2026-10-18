//! FHIR R5 router.
//!
//! Phase 2 stub: every route is wired to a handler that returns
//! `503 + transient OperationOutcome`. Phase 3 replaces the handler
//! bodies (in `handlers.rs`) with real proxy / direct-read
//! implementations — the route table itself is stable.

use axum::{
    Router,
    routing::{get, post},
};

use crate::api::rest::AppState;

/// Build the FHIR R5 sub-router and attach the shared [`AppState`].
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/fhir", post(super::handlers::post_bundle))
        .route(
            "/fhir/Patient",
            post(super::handlers::create_patient).get(super::handlers::list_patients),
        )
        .route(
            "/fhir/Patient/{id}",
            get(super::handlers::get_patient)
                .put(super::handlers::update_patient)
                .delete(super::handlers::delete_patient),
        )
        .route("/fhir/Encounter", post(super::handlers::create_encounter))
        .route("/fhir/Encounter/{id}", get(super::handlers::get_encounter))
        .route(
            "/fhir/Appointment",
            post(super::handlers::create_appointment),
        )
        .route(
            "/fhir/Appointment/{id}",
            get(super::handlers::get_appointment),
        )
        .route(
            "/fhir/Practitioner",
            post(super::handlers::create_practitioner),
        )
        .route(
            "/fhir/Practitioner/{id}",
            get(super::handlers::get_practitioner)
                .put(super::handlers::update_practitioner)
                .delete(super::handlers::delete_practitioner),
        )
        .route("/fhir/Schedule", post(super::handlers::create_schedule))
        .route(
            "/fhir/Schedule/{id}",
            get(super::handlers::get_schedule)
                .put(super::handlers::update_schedule)
                .delete(super::handlers::delete_schedule),
        )
        .route("/fhir/Slot", post(super::handlers::create_slot))
        .route(
            "/fhir/Slot/{id}",
            get(super::handlers::get_slot)
                .put(super::handlers::update_slot)
                .delete(super::handlers::delete_slot),
        )
        .route("/fhir/Location/{id}", get(super::handlers::get_location))
        .route("/fhir/Coverage/{id}", get(super::handlers::get_coverage))
        .with_state(state)
}
