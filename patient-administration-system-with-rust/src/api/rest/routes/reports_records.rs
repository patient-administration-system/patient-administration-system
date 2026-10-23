//! Part of [`super`]: reports records.

use super::*;

/// Support flags, follow-up, the no-show score, identity proofing and experiments.
pub(super) fn support() -> Router<AppState> {
    Router::new()
        .route(
            "/api/identity-proofings/{subject}",
            put(handlers_identity::record)
                .get(handlers_identity::get)
                .delete(handlers_identity::revoke),
        )
        .route(
            "/api/identity-proofings/{subject}/check",
            get(handlers_identity::check),
        )
        .route(
            "/api/experiments",
            post(handlers_experiments::start).get(handlers_experiments::list),
        )
        .route(
            "/api/experiments/{id}/stop",
            post(handlers_experiments::stop),
        )
        .route(
            "/api/experiments/{id}/readout",
            get(handlers_experiments::readout),
        )
        .route("/api/noshow/status", get(handlers_noshow::status))
        .route("/api/noshow/rollout", get(handlers_noshow::rollout))
        .route(
            "/api/noshow/rollout/{service_type}",
            put(handlers_noshow::set_stage),
        )
        .route("/api/noshow/evaluation", get(handlers_noshow::evaluation))
        .route("/api/noshow/model-card", get(handlers_noshow::model_card))
        .route("/api/noshow/reviews", post(handlers_noshow::record_review))
        .route(
            "/api/noshow/fairness-audit",
            get(handlers_noshow::fairness_audit),
        )
        .route(
            "/api/noshow/recalculate",
            post(handlers_noshow::recalculate),
        )
        .route(
            "/api/noshow/for-appointments",
            get(handlers_noshow::for_appointments),
        )
        .route(
            "/api/noshow/priority-list",
            get(handlers_noshow::priority_list),
        )
        .route("/api/support-flags", post(handlers_support::add))
        .route("/api/support-flags/open", get(handlers_support::open))
        .route(
            "/api/support-flags/{id}/resolve",
            post(handlers_support::resolve),
        )
        .route("/api/follow-up-policies", get(handlers_followup::policies))
        .route(
            "/api/follow-up-policies/{service_type}",
            put(handlers_followup::approve_policy),
        )
        .route(
            "/api/follow-up-tasks/open",
            get(handlers_followup::open_tasks),
        )
        .route(
            "/api/follow-up-tasks/{id}/outcome",
            post(handlers_followup::close_task),
        )
}

/// Analytics, reports, tolerances, audit and administration.
pub(super) fn reporting() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(handlers::health))
        .route(
            "/api/analytics/appointment-outcomes",
            get(handlers_analytics::outcomes),
        )
        .route(
            "/api/analytics/refill-rates",
            get(handlers_analytics::refill_rates),
        )
        .route(
            "/api/analytics/deprivation-lookup",
            put(handlers_analytics::load_deprivation_lookup),
        )
        .route(
            "/api/analytics/demographics/refresh",
            post(handlers_analytics::refresh_demographics),
        )
        .route(
            "/api/analytics/demographics/{person_id}",
            put(handlers_analytics::set_demographics),
        )
        .route(
            "/api/reports/clinic-activity",
            get(handlers_reports::clinic_activity),
        )
        .route(
            "/api/reports/follow-up-delay",
            get(handlers_follow_up_delay::delay_report),
        )
        .route("/api/reports/pifu", get(handlers_pifu::uptake))
        .route("/api/tolerances", get(handlers_tolerance::list))
        .route("/api/tolerances/status", get(handlers_tolerance::status))
        .route(
            "/api/tolerances/evaluate",
            post(handlers_tolerance::evaluate),
        )
        .route("/api/tolerance-alerts", get(handlers_tolerance::alerts))
        .route(
            "/api/tolerance-alerts/{id}/acknowledge",
            post(handlers_tolerance::acknowledge),
        )
        .route(
            "/api/tolerances/{metric}",
            put(handlers_tolerance::set).delete(handlers_tolerance::reset),
        )
        // Audit query.
        .route("/api/audit/recent", get(handlers_audit::list_recent))
        .route("/api/audit/entity", get(handlers_audit::list_for_entity))
        .route("/api/admin/metrics", get(handlers_admin::metrics))
        // Admin: outbox diagnostics + DLQ replay.
        .route(
            "/api/admin/outbox/unpublished",
            get(handlers_admin::list_unpublished_outbox),
        )
        .route(
            "/api/admin/outbox/dead-letters",
            get(handlers_admin::list_dead_letters),
        )
        .route(
            "/api/admin/outbox/dead-letters/{id}/replay",
            post(handlers_admin::replay_dead_letter),
        )
}

/// Facilities, wards, beds, billing, coverage and consent.
pub(super) fn records() -> Router<AppState> {
    Router::new()
        // Resources — ward occupancy + bed reads + bed status change.
        .route(
            "/api/wards/{id}/occupancy",
            get(handlers_resources::ward_occupancy),
        )
        .route(
            "/api/wards/{id}/beds",
            get(handlers_resources::list_beds_by_ward),
        )
        .route("/api/beds/{id}", get(handlers_resources::get_bed))
        .route(
            "/api/beds/{id}/status",
            put(handlers_resources::set_bed_status),
        )
        // Billing.
        .route("/api/accounts", post(handlers_billing::open_account))
        .route(
            "/api/accounts/{id}/close",
            post(handlers_billing::close_account),
        )
        .route("/api/charges", post(handlers_billing::post_charge))
        .route("/api/invoices", post(handlers_billing::finalize_invoice))
        .route("/api/payments", post(handlers_billing::post_payment))
        // Coverage.
        .route("/api/coverages", post(handlers_coverage::create_coverage))
        .route(
            "/api/coverages/{id}",
            get(handlers_coverage::get_coverage)
                .put(handlers_coverage::update_coverage)
                .delete(handlers_coverage::delete_coverage),
        )
        .route(
            "/api/accounts/{id}/coverages",
            get(handlers_coverage::list_coverages_for_account),
        )
        .route(
            "/api/consents/{id}/revoke",
            post(handlers_consent::revoke_consent),
        )
        // Facility / Ward / Room / Bed creation + reads.
        .route(
            "/api/facilities",
            post(handlers_resources::create_facility).get(handlers_resources::list_facilities),
        )
        .route(
            "/api/facilities/{id}",
            get(handlers_resources::get_facility),
        )
        .route(
            "/api/facilities/{id}/wards",
            get(handlers_resources::list_wards_by_facility),
        )
        .route("/api/wards", post(handlers_resources::create_ward))
        .route("/api/wards/{id}", get(handlers_resources::get_ward))
        .route(
            "/api/wards/{id}/rooms",
            get(handlers_resources::list_rooms_by_ward),
        )
        .route("/api/rooms", post(handlers_resources::create_room))
        .route("/api/rooms/{id}", get(handlers_resources::get_room))
        .route("/api/beds", post(handlers_resources::create_bed))
}
