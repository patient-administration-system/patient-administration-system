//! Part of [`super`]: clinical.

use super::*;

/// Admissions, transfers, discharges, leave and the other ADT events.
pub(super) fn adt() -> Router<AppState> {
    Router::new()
        // ADT — admit / transfer / discharge / cancel-admit / cancel-discharge.
        .route("/api/admissions", post(handlers_adt::admit))
        .route(
            "/api/admissions/{id}/transfer",
            post(handlers_adt::transfer),
        )
        .route(
            "/api/admissions/{id}/discharge",
            post(handlers_adt::discharge),
        )
        .route(
            "/api/admissions/{id}/cancel-admit",
            post(handlers_adt::cancel_admit),
        )
        .route(
            "/api/admissions/{id}/cancel-discharge",
            post(handlers_adt::cancel_discharge),
        )
        // Extra ADT variants — pre-admit / cancel-pre-admit / cancel-transfer
        // / leave-start / leave-end / change-to-{inpatient,outpatient}.
        .route("/api/admissions/pre-admit", post(handlers_adt::pre_admit))
        .route(
            "/api/admissions/cancel-pre-admit",
            post(handlers_adt::cancel_pre_admit),
        )
        .route(
            "/api/admissions/change-to-inpatient",
            post(handlers_adt::change_to_inpatient),
        )
        .route(
            "/api/admissions/{id}/change-to-outpatient",
            post(handlers_adt::change_to_outpatient),
        )
        .route(
            "/api/admissions/{id}/cancel-transfer",
            post(handlers_adt::cancel_transfer),
        )
        .route(
            "/api/admissions/{id}/leave-start",
            post(handlers_adt::leave_start),
        )
        .route(
            "/api/admissions/{id}/leave-end",
            post(handlers_adt::leave_end),
        )
        // Encounter.
        .route(
            "/api/encounters/{id}",
            get(handlers_encounter::get_encounter),
        )
        .route(
            "/api/encounters/{id}/status",
            put(handlers_encounter::set_status),
        )
        .route(
            "/api/encounters/{id}/triage",
            get(handlers_triage::list).post(handlers_triage::record),
        )
        .route(
            "/api/encounters/{id}/cancel",
            post(handlers_encounter::cancel_encounter),
        )
}

/// Schedules, slots, appointments, series, rebooking, virtual visits and transport.
pub(super) fn scheduling() -> Router<AppState> {
    Router::new()
        .route(
            "/api/slots/{id}/capacity",
            put(handlers_group_sessions::set_capacity),
        )
        .route(
            "/api/slots/{id}/attendees",
            get(handlers_group_sessions::attendees),
        )
        .route("/api/pifu/{id}/request", post(handlers_pifu::request))
        .route("/api/pifu/{id}/review", post(handlers_pifu::review))
        .route("/api/pifu/requests", get(handlers_pifu::requests))
        .route(
            "/api/pifu/requests/{id}/handle",
            post(handlers_pifu::handle),
        )
        .route("/api/pifu/due-reviews", get(handlers_pifu::due_reviews))
        .route(
            "/api/interpreter-requests/{id}/confirm",
            post(handlers_interpreter::confirm),
        )
        .route(
            "/api/interpreter-requests/unconfirmed",
            get(handlers_interpreter::unconfirmed),
        )
        // Scheduling — book / cancel / check-in / complete.
        .route("/api/slots/{id}/book", post(handlers_scheduling::book_slot))
        .route(
            "/api/appointments",
            get(handlers_scheduling::list_appointments),
        )
        .route(
            "/api/appointments/{id}/cancel",
            post(handlers_scheduling::cancel_appointment),
        )
        .route(
            "/api/appointments/{id}/rebook-options",
            get(handlers_scheduling::rebook_options),
        )
        .route(
            "/api/appointments/{id}/rebook",
            post(handlers_scheduling::rebook_appointment),
        )
        .route(
            "/api/appointments/{id}/history",
            get(handlers_history::appointment_history),
        )
        .route(
            "/api/appointments/{id}/lead-time",
            get(handlers_waits::lead_time),
        )
        .route(
            "/api/appointments/{id}/transport",
            get(handlers_transport::booking),
        )
        .route(
            "/api/appointments/{id}/noshow",
            get(handlers_noshow::for_appointment),
        )
        .route(
            "/api/virtual-ok-services",
            get(handlers_virtual::list_services),
        )
        .route(
            "/api/virtual-ok-services/{service_type}",
            put(handlers_virtual::approve_service).delete(handlers_virtual::remove_service),
        )
        .route("/api/conversions/pending", get(handlers_virtual::pending))
        .route(
            "/api/appointments/{id}/video-failed",
            post(handlers_virtual::video_failed),
        )
        .route(
            "/api/appointments/{id}/conversion/{decision}",
            post(handlers_virtual::review),
        )
        .route(
            "/api/appointments/{id}/location",
            put(handlers_scheduling::move_to_room),
        )
        .route(
            "/api/facility/in-facility",
            get(handlers_scheduling::in_facility),
        )
        .route(
            "/api/appointments/{id}/barrier",
            put(handlers_scheduling::record_barrier),
        )
        .route(
            "/api/appointments/{id}/check-in",
            post(handlers_scheduling::check_in_appointment),
        )
        .route(
            "/api/appointments/{id}/complete",
            post(handlers_scheduling::complete_appointment),
        )
        // Scheduling — no-show.
        .route(
            "/api/appointments/{id}/no-show",
            post(handlers_scheduling::mark_no_show),
        )
        // Appointment series.
        .route(
            "/api/appointment-series",
            post(handlers_series::create_series),
        )
        .route(
            "/api/appointment-series/{id}",
            get(handlers_series::get_series),
        )
        .route(
            "/api/appointment-series/{id}/cancel",
            post(handlers_series::cancel_series),
        )
}
