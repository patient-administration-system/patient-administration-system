//! REST API routing table — Phase 3b1.
//!
//! Wires:
//! - `/api/health` (real)
//! - `/api/patients/*` (proxy `person-service`)
//! - `/api/practitioners/*` (proxy `worker-service`)
//! - `/api/upstream/{person,worker}-merged` (inbound merge webhooks)
//! - `/api/admissions` + `/api/admissions/:id/{transfer,discharge,cancel-admit}` (ADT)
//! - `/api/slots/:id/book` + `/api/appointments/:id/{cancel,check-in,complete}` (scheduling)
//! - `/api/wards/:id/{occupancy,beds}` + `/api/beds/:id` + `PUT /api/beds/:id/status` (resources)
//!
//! Still unbound (Phase 3b2): billing, communication, consent, coverage,
//! waitlist + RTT, encounter CRUD, audit query, outbox diagnostics +
//! DLQ, dashboard, facility/ward/room creation, bulk slot generation,
//! appointment series, the remaining ADT cancellation + leave + change
//! variants.

use axum::{
    Router,
    routing::{delete, get, post, put},
};

use super::AppState;
use super::handlers;
use super::handlers_admin;
use super::handlers_adt;
use super::handlers_audit;
use super::handlers_billing;
use super::handlers_consent;
use super::handlers_contact;
use super::handlers_coverage;
use super::handlers_encounter;
use super::handlers_followup;
use super::handlers_history;
use super::handlers_hl7v2;
use super::handlers_letters;
use super::handlers_links;
use super::handlers_resources;
use super::handlers_scheduling;
use super::handlers_series;
use super::handlers_standby;
use super::handlers_support;
use super::handlers_virtual;
use super::handlers_waitlist;
use super::handlers_waits;
use super::handlers_writeback;
use super::upstream_proxy;

/// Build the REST sub-router and attach the shared [`AppState`].
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(handlers::health))
        // /api/patients/* — proxy to person-service
        .route(
            "/api/patients",
            post(upstream_proxy::create_patient).get(upstream_proxy::search_patients),
        )
        .route(
            "/api/patients/{id}",
            get(upstream_proxy::get_patient)
                .put(upstream_proxy::update_patient)
                .delete(upstream_proxy::delete_patient),
        )
        .route("/api/patients/search", get(upstream_proxy::search_patients))
        .route(
            "/api/patients/{id}/masked",
            get(upstream_proxy::masked_patient),
        )
        .route(
            "/api/patients/{id}/export",
            get(upstream_proxy::export_patient),
        )
        .route(
            "/api/patients/{id}/audit",
            get(upstream_proxy::audit_patient),
        )
        // Phase 3d bulk interchange — transparent passthrough to person-service.
        .route(
            "/api/patients/export.json",
            get(upstream_proxy::export_patients_json),
        )
        .route(
            "/api/patients/export.xml",
            get(upstream_proxy::export_patients_xml),
        )
        .route(
            "/api/patients/export.tsv",
            get(upstream_proxy::export_patients_tsv),
        )
        .route(
            "/api/patients/export.csv",
            get(upstream_proxy::export_patients_csv),
        )
        .route(
            "/api/patients/import",
            post(upstream_proxy::import_patients),
        )
        // /api/practitioners/* — proxy to worker-service
        .route(
            "/api/practitioners",
            post(upstream_proxy::create_practitioner).get(upstream_proxy::search_practitioners),
        )
        .route(
            "/api/practitioners/{id}",
            get(upstream_proxy::get_practitioner)
                .put(upstream_proxy::update_practitioner)
                .delete(upstream_proxy::delete_practitioner),
        )
        .route(
            "/api/practitioners/search",
            get(upstream_proxy::search_practitioners),
        )
        .route(
            "/api/practitioners/{id}/masked",
            get(upstream_proxy::masked_practitioner),
        )
        .route(
            "/api/practitioners/{id}/export",
            get(upstream_proxy::export_practitioner),
        )
        .route(
            "/api/practitioners/{id}/audit",
            get(upstream_proxy::audit_practitioner),
        )
        // Inbound merge webhooks (HMAC-verified inside the handler).
        .route(
            "/api/upstream/person-merged",
            post(upstream_proxy::person_merged_webhook),
        )
        .route(
            "/api/upstream/worker-merged",
            post(upstream_proxy::worker_merged_webhook),
        )
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
        // Scheduling — book / cancel / check-in / complete.
        .route("/api/slots/{id}/book", post(handlers_scheduling::book_slot))
        // Contact reliability and communication needs (CAP-4).
        .route(
            "/api/patients/{id}/communication-profile",
            get(handlers_contact::get_profile).put(handlers_contact::put_profile),
        )
        .route(
            "/api/patients/{id}/communication-profile/verify-mobile",
            post(handlers_contact::verify_mobile),
        )
        .route(
            "/api/patients/{id}/channel-optouts",
            get(handlers_contact::list_optouts).post(handlers_contact::add_optout),
        )
        .route(
            "/api/patients/{id}/channel-optouts/{channel}",
            axum::routing::delete(handlers_contact::remove_optout),
        )
        .route(
            "/api/communication-profiles/stale",
            get(handlers_contact::stale_profiles),
        )
        // Released slots and the standby list (CAP-3).
        .route(
            "/api/standby",
            post(handlers_standby::add_standby).get(handlers_standby::list_standby),
        )
        .route(
            "/api/standby/{id}",
            delete(handlers_standby::remove_standby),
        )
        .route(
            "/api/standby/{id}/book",
            post(handlers_standby::book_from_standby),
        )
        .route("/api/slot-releases", get(handlers_standby::list_releases))
        .route(
            "/api/slot-releases/{id}/candidates",
            get(handlers_standby::release_candidates),
        )
        // Write-back (CAP-2): a patient's reply changes the appointment now.
        .route(
            "/api/write-back/replies",
            post(handlers_writeback::post_reply),
        )
        .route(
            "/api/write-back/batch",
            post(handlers_writeback::post_batch),
        )
        .route("/api/reply-links/{token}", get(handlers_links::get_link))
        .route(
            "/api/reply-links/{token}/rebook",
            post(handlers_links::post_link_rebook),
        )
        .route(
            "/api/reply-links/{token}/{action}",
            post(handlers_links::post_link_action),
        )
        .route(
            "/api/staff-call-tasks/open",
            get(handlers_links::open_calls),
        )
        .route(
            "/api/staff-call-tasks/{id}/outcome",
            post(handlers_links::call_outcome),
        )
        .route(
            "/api/letter-template-groups",
            post(handlers_letters::create_group),
        )
        .route(
            "/api/letter-templates/check-sms",
            post(handlers_letters::check_sms),
        )
        .route("/api/write-back/texts", post(handlers_writeback::post_text))
        .route(
            "/api/inbound-texts/open",
            get(handlers_writeback::open_texts),
        )
        .route(
            "/api/inbound-texts/{id}/handled",
            post(handlers_writeback::text_handled),
        )
        .route(
            "/api/appointments",
            get(handlers_scheduling::list_appointments),
        )
        .route(
            "/api/patients/{id}/appointments",
            get(handlers_scheduling::list_patient_appointments),
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
            "/api/patients/{id}/contact-log",
            get(handlers_history::contact_log),
        )
        .route(
            "/api/appointments/{id}/lead-time",
            get(handlers_waits::lead_time),
        )
        .route(
            "/api/patients/{id}/waits",
            get(handlers_waits::patient_waits),
        )
        .route(
            "/api/pathways/due-check",
            get(handlers_waits::due_for_check),
        )
        .route(
            "/api/pathways/{id}/still-needed-checks",
            post(handlers_waits::ask),
        )
        .route(
            "/api/still-needed-checks/{id}/answer",
            post(handlers_waits::answer),
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
            "/api/appointments/{id}/conversion/{decision}",
            post(handlers_virtual::review),
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
        .route(
            "/api/patients/{id}/appointment-series",
            get(handlers_series::list_series_for_person),
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
            "/api/encounters/{id}/cancel",
            post(handlers_encounter::cancel_encounter),
        )
        .route(
            "/api/patients/{id}/encounters",
            get(handlers_encounter::list_encounters_for_person),
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
            "/api/patients/{id}/coverages",
            get(handlers_coverage::list_coverages_for_person),
        )
        .route(
            "/api/accounts/{id}/coverages",
            get(handlers_coverage::list_coverages_for_account),
        )
        // Consent.
        .route(
            "/api/patients/{id}/consents",
            post(handlers_consent::create_consent).get(handlers_consent::list_consents_for_person),
        )
        .route(
            "/api/consents/{id}/revoke",
            post(handlers_consent::revoke_consent),
        )
        // Waitlist + RTT.
        .route(
            "/api/waitlist",
            post(handlers_waitlist::add_to_waitlist).get(handlers_waitlist::list_waitlist),
        )
        .route(
            "/api/waitlist/{id}",
            delete(handlers_waitlist::remove_from_waitlist),
        )
        .route(
            "/api/waitlist/{id}/book",
            post(handlers_waitlist::book_from_waitlist),
        )
        .route("/api/rtt/start", post(handlers_waitlist::rtt_start))
        .route("/api/rtt/{id}/pause", post(handlers_waitlist::rtt_pause))
        .route("/api/rtt/{id}/resume", post(handlers_waitlist::rtt_resume))
        .route("/api/rtt/{id}/stop", post(handlers_waitlist::rtt_stop))
        .route(
            "/api/rtt/{id}/weeks-waiting",
            get(handlers_waitlist::rtt_weeks_waiting),
        )
        .route(
            "/api/rtt/{id}/breaching",
            get(handlers_waitlist::rtt_is_breaching),
        )
        // Audit query.
        .route("/api/audit/recent", get(handlers_audit::list_recent))
        .route("/api/audit/entity", get(handlers_audit::list_for_entity))
        .route(
            "/api/patients/{id}/audit/local",
            get(handlers_audit::list_local_for_person),
        )
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
        // Letter templates + generation.
        .route(
            "/api/letter-templates",
            post(handlers_letters::create_template).get(handlers_letters::list_templates),
        )
        .route(
            "/api/letter-templates/{id}",
            get(handlers_letters::get_template).put(handlers_letters::update_template),
        )
        .route(
            "/api/letters/generate",
            post(handlers_letters::generate_letter),
        )
        .route(
            "/api/patients/{id}/letters",
            get(handlers_letters::list_patient_letters),
        )
        .route("/api/letters/{id}", get(handlers_letters::get_letter))
        .route("/api/letters/{id}/sent", post(handlers_letters::mark_sent))
        .route(
            "/api/letters/{id}/failed",
            post(handlers_letters::mark_failed),
        )
        // HL7 v2 (Phase 3c minimum viable surface: parse + ADT
        // A28/A01/A02/A03). Each handler dedup-or-creates the person
        // upstream via `PersonClient` before invoking the local
        // ADT service. Phase 3c2 lands the remaining 14 ADT variants
        // + DFT^P03 + SIU + MFN.
        .route("/api/hl7/v2/parse", post(handlers_hl7v2::parse))
        .route("/api/hl7/v2/patient", post(handlers_hl7v2::add_person))
        .route("/api/hl7/v2/admit", post(handlers_hl7v2::admit))
        .route("/api/hl7/v2/transfer", post(handlers_hl7v2::transfer))
        .route("/api/hl7/v2/discharge", post(handlers_hl7v2::discharge))
        // Phase 3c2 — remaining 13 ADT trigger surface (URLs match
        // `listener.rs::route_for_payload` so MLLP traffic reaches
        // them unchanged).
        .route("/api/hl7/v2/register", post(handlers_hl7v2::register))
        .route("/api/hl7/v2/pre-admit", post(handlers_hl7v2::pre_admit))
        .route(
            "/api/hl7/v2/cancel-pre-admit",
            post(handlers_hl7v2::cancel_pre_admit),
        )
        .route(
            "/api/hl7/v2/change-to-inpatient",
            post(handlers_hl7v2::change_to_inpatient),
        )
        .route(
            "/api/hl7/v2/change-to-outpatient",
            post(handlers_hl7v2::change_to_outpatient),
        )
        .route("/api/hl7/v2/update", post(handlers_hl7v2::update))
        .route(
            "/api/hl7/v2/cancel-admit",
            post(handlers_hl7v2::cancel_admit),
        )
        .route(
            "/api/hl7/v2/cancel-transfer",
            post(handlers_hl7v2::cancel_transfer),
        )
        .route(
            "/api/hl7/v2/cancel-discharge",
            post(handlers_hl7v2::cancel_discharge),
        )
        .route("/api/hl7/v2/leave-start", post(handlers_hl7v2::leave_start))
        .route("/api/hl7/v2/leave-end", post(handlers_hl7v2::leave_end))
        .route(
            "/api/hl7/v2/delete-patient",
            post(handlers_hl7v2::delete_patient),
        )
        .route("/api/hl7/v2/merge", post(handlers_hl7v2::merge))
        // Phase 3c3 — DFT / SIU / MFN / batch envelope.
        .route("/api/hl7/v2/dft", post(handlers_hl7v2::dft))
        .route(
            "/api/hl7/v2/schedule-book",
            post(handlers_hl7v2::schedule_book),
        )
        .route(
            "/api/hl7/v2/schedule-reschedule",
            post(handlers_hl7v2::schedule_reschedule),
        )
        .route(
            "/api/hl7/v2/schedule-modify",
            post(handlers_hl7v2::schedule_modify),
        )
        .route(
            "/api/hl7/v2/schedule-cancel",
            post(handlers_hl7v2::schedule_cancel),
        )
        .route("/api/hl7/v2/mfn-staff", post(handlers_hl7v2::mfn_staff))
        .route(
            "/api/hl7/v2/mfn-location",
            post(handlers_hl7v2::mfn_location),
        )
        .route("/api/hl7/v2/batch", post(handlers_hl7v2::batch))
        // Dashboard (/dashboard + 4 fragment routes) is mounted from
        // `main.rs` as a sibling router that merges in at boot time —
        // it isn't part of this REST table.
        .with_state(state)
}
