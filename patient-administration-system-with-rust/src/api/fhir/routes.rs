//! FHIR R5 router: the route table for the handlers in `handlers.rs`.

use axum::{
    Router,
    routing::{get, post},
};

use crate::api::rest::AppState;

/// Build the FHIR R5 sub-router and attach the shared [`AppState`].
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/fhir", post(super::handlers::post_bundle))
        // What the server supports; `capability::SUPPORTED` lists every route below (NEW-21).
        .route("/fhir/metadata", get(super::handlers::metadata))
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
        .route("/fhir/Encounter/{id}", get(super::handlers::get_encounter))
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
        .route("/fhir/Flag", get(super::handlers::search_flags))
        .route(
            "/fhir/SubscriptionTopic",
            get(super::handlers::search_subscription_topics),
        )
        .route(
            "/fhir/Subscription",
            get(super::handlers::search_subscriptions),
        )
        // IHE ITI Scheduling is FHIR R4: its own prefix and CapabilityStatement (NEW-10).
        .route("/fhir/r4/metadata", get(super::ihe_scheduling::metadata))
        .route(
            "/fhir/r4/Appointment/$find",
            post(super::ihe_scheduling::find),
        )
        .route(
            "/fhir/r4/Appointment/$hold",
            post(super::ihe_scheduling::hold),
        )
        .route(
            "/fhir/r4/Appointment/$book",
            post(super::ihe_scheduling::book),
        )
        .route(
            "/fhir/Observation",
            get(super::handlers::search_observations),
        )
        .route("/fhir/Coverage/{id}", get(super::handlers::get_coverage))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::api::rest::read_audit::read_audit_middleware,
        ))
        .with_state(state)
}
