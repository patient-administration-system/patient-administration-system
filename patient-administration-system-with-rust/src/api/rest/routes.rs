//! The REST API's routing table: `router` merges one function per domain (people, workforce, ADT,
//! scheduling, messaging, waiting, support, reporting, records, HL7 v2), each listing its own routes
//! (QUAL-30). Every route also carries a `#[utoipa::path]` on its handler, and the OpenAPI test fails when
//! a route here is missing from the document. All methods of one path live in the same function.
//!
//! The operator dashboard (`/dashboard` and its fragments) is merged in by `main.rs`, not here.

// One function per domain, each short enough to read (QUAL-30).
#![deny(clippy::too_many_lines)]

use axum::{
    Router,
    routing::{delete, get, post, put},
};

use super::AppState;
use super::handlers;
use super::handlers_admin;
use super::handlers_adt;
use super::handlers_analytics;
use super::handlers_audit;
use super::handlers_billing;
use super::handlers_consent;
use super::handlers_contact;
use super::handlers_coverage;
use super::handlers_encounter;
use super::handlers_experiments;
use super::handlers_followup;
use super::handlers_history;
use super::handlers_hl7v2;
use super::handlers_identity;
use super::handlers_ivr;
use super::handlers_letters;
use super::handlers_links;
use super::handlers_noshow;
use super::handlers_offers;
use super::handlers_print;
use super::handlers_receipts;
use super::handlers_referrals;
use super::handlers_reports;
use super::handlers_resources;
use super::handlers_scheduling;
use super::handlers_series;
use super::handlers_standby;
use super::handlers_support;
use super::handlers_tolerance;
use super::handlers_transport;
use super::handlers_virtual;
use super::handlers_waitlist;
use super::handlers_waits;
use super::handlers_whatsapp;
use super::handlers_writeback;
use super::upstream_proxy;

/// Build the REST sub-router and attach the shared [`AppState`].
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(people())
        .merge(workforce())
        .merge(adt())
        .merge(scheduling())
        .merge(messaging())
        .merge(waiting())
        .merge(support())
        .merge(reporting())
        .merge(records())
        .merge(hl7v2())
        // Dashboard (/dashboard + 4 fragment routes) is mounted from
        // `main.rs` as a sibling router that merges in at boot time —
        // it isn't part of this REST table.
        .with_state(state)
}

/// Everything under `/api/patients`: the person-service proxy and the PAS's own per-patient records; the merge webhooks.
fn people() -> Router<AppState> {
    Router::new()
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
        // Inbound merge webhooks (HMAC-verified inside the handler).
        .route(
            "/api/upstream/person-merged",
            post(upstream_proxy::person_merged_webhook),
        )
        .route(
            "/api/upstream/worker-merged",
            post(upstream_proxy::worker_merged_webhook),
        )
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
            "/api/patients/{id}/appointments",
            get(handlers_scheduling::list_patient_appointments),
        )
        .route(
            "/api/patients/{id}/contact-log",
            get(handlers_history::contact_log),
        )
        .route(
            "/api/patients/{id}/waits",
            get(handlers_waits::patient_waits),
        )
        .route(
            "/api/patients/{id}/transport-eligibility",
            put(handlers_transport::set_eligible).delete(handlers_transport::withdraw_eligible),
        )
        .route(
            "/api/patients/{id}/transport-consent",
            put(handlers_transport::record_consent).delete(handlers_transport::withdraw_consent),
        )
        .route(
            "/api/validate/nhs-number",
            post(handlers_contact::validate_nhs_number),
        )
        .route(
            "/api/patients/{id}/whatsapp-optin",
            put(handlers_whatsapp::record_optin).delete(handlers_whatsapp::withdraw_optin),
        )
        .route(
            "/api/patients/{id}/appointment-series",
            get(handlers_series::list_series_for_person),
        )
        .route(
            "/api/patients/{id}/encounters",
            get(handlers_encounter::list_encounters_for_person),
        )
        .route(
            "/api/patients/{id}/coverages",
            get(handlers_coverage::list_coverages_for_person),
        )
        // Consent.
        .route(
            "/api/patients/{id}/consents",
            post(handlers_consent::create_consent).get(handlers_consent::list_consents_for_person),
        )
        .route(
            "/api/patients/{id}/audit/local",
            get(handlers_audit::list_local_for_person),
        )
        .route(
            "/api/patients/{id}/letters",
            get(handlers_letters::list_patient_letters),
        )
}

/// The worker-service proxy under `/api/practitioners`.
fn workforce() -> Router<AppState> {
    Router::new()
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
}

/// Admissions, transfers, discharges, leave and the other ADT events.
fn adt() -> Router<AppState> {
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
            "/api/encounters/{id}/cancel",
            post(handlers_encounter::cancel_encounter),
        )
}

/// Schedules, slots, appointments, series, rebooking, virtual visits and transport.
fn scheduling() -> Router<AppState> {
    Router::new()
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

/// Letters, templates, replies, links, texts, receipts, printing and contact.
fn messaging() -> Router<AppState> {
    Router::new()
        .route(
            "/api/communication-profiles/stale",
            get(handlers_contact::stale_profiles),
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
        .route("/api/offer-links/{token}", get(handlers_offers::get_offer))
        .route(
            "/api/offer-links/{token}/{action}",
            post(handlers_offers::post_offer_action),
        )
        .route("/api/print-queue", get(handlers_print::queue))
        .route("/api/print-batches", post(handlers_print::export))
        .route(
            "/api/print-batches/{id}/sent",
            post(handlers_print::mark_sent),
        )
        .route(
            "/api/write-back/receipts",
            post(handlers_receipts::post_receipt),
        )
        .route(
            "/api/letters/{id}/receipts",
            get(handlers_receipts::letter_receipts),
        )
        .route(
            "/api/contact-problems",
            get(handlers_receipts::contact_problems),
        )
        .route(
            "/api/contact-problems/{person_id}/resolve",
            post(handlers_receipts::resolve_problem),
        )
        .route(
            "/api/letter-templates/{id}/whatsapp-approval",
            put(handlers_whatsapp::approve_template).delete(handlers_whatsapp::revoke_template),
        )
        .route("/api/write-back/ivr/menu", post(handlers_ivr::menu))
        .route("/api/write-back/ivr/press", post(handlers_ivr::press))
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
            "/api/letters/preview",
            post(handlers_letters::preview_letter),
        )
        .route(
            "/api/letters/generate",
            post(handlers_letters::generate_letter),
        )
        .route("/api/letters/{id}", get(handlers_letters::get_letter))
        .route("/api/letters/{id}/sent", post(handlers_letters::mark_sent))
        .route(
            "/api/letters/{id}/failed",
            post(handlers_letters::mark_failed),
        )
}

/// Waiting lists, RTT, standby, released slots, referrals and long waits.
fn waiting() -> Router<AppState> {
    Router::new()
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
        .route(
            "/api/slot-releases/{id}/offers",
            get(handlers_offers::release_offers),
        )
        .route(
            "/api/slot-releases/{id}/offer",
            post(handlers_offers::offer_now),
        )
        .route(
            "/api/referrals",
            post(handlers_referrals::create).get(handlers_referrals::list),
        )
        .route("/api/referrals/{id}", get(handlers_referrals::get))
        .route(
            "/api/referrals/{id}/book-first-available",
            post(handlers_referrals::book_first_available),
        )
        .route(
            "/api/referrals/{id}/waitlist",
            post(handlers_referrals::waitlist),
        )
        .route("/api/referrals/{id}/close", post(handlers_referrals::close))
        .route(
            "/api/referrals/{id}/triage",
            post(handlers_referrals::triage),
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
}

/// Support flags, follow-up, the no-show score, identity proofing and experiments.
fn support() -> Router<AppState> {
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
fn reporting() -> Router<AppState> {
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
fn records() -> Router<AppState> {
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

/// HL7 v2 messages over HTTP (the MLLP listener posts to these paths).
fn hl7v2() -> Router<AppState> {
    Router::new()
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
}
