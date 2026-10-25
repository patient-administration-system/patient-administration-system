//! OpenAPI document: every route in `api/rest/routes.rs` and `api/fhir/routes.rs`, by path and method.
//!
//! Each routed handler carries a `#[utoipa::path]` (path, method, tag, path parameters and the three response
//! classes); this file lists them. **What it does not describe yet:** request bodies, query parameters and the
//! typed `data` of each response (handlers return `ApiResponse<serde_json::Value>`), and which routes are
//! public or need a token. `tests` below fail when a route is added without being documented, and when
//! `spec/openapi.json` is stale (regenerate with `UPDATE_OPENAPI=1 cargo test -p patient-administration-system
//! openapi`).

use utoipa::OpenApi;

use crate::api::fhir::resources::{
    FhirBundleRequest, FhirBundleResponse, FhirBundleWriteEntry, FhirWriteBundle,
};

/// The same handler answers `/api/patients/search`: documented here because one function can carry one path.
#[utoipa::path(
    get,
    path = "/api/patients/search",
    tag = "Patients",
    summary = "Search (alias of the list route)",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
#[allow(
    dead_code,
    reason = "only its #[utoipa::path] is used: one function documents one path"
)]
fn alias_api_patients_search_get() {}

/// The same handler answers `/api/practitioners/search`: documented here because one function can carry one path.
#[utoipa::path(
    get,
    path = "/api/practitioners/search",
    tag = "Practitioners",
    summary = "Search (alias of the list route)",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
#[allow(
    dead_code,
    reason = "only its #[utoipa::path] is used: one function documents one path"
)]
fn alias_api_practitioners_search_get() {}

/// Adds the credentials and says which routes need which. Every route needs a bearer token (a PASETO v4
/// public token, or the legacy shared token where that is still allowed) except: `/api/health` (open); the
/// signed merge webhooks under `/api/upstream/` (an HMAC of the body in `x-upstream-signature`); and the
/// message carriers' write-backs under `/api/write-back/`, which take a bearer token or the carrier's own
/// signed credential (`x-carrier-id`, `x-carrier-timestamp`, `x-carrier-signature`).
struct Credentials;

impl utoipa::Modify for Credentials {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{
            ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme,
        };
        let c = openapi.components.get_or_insert_with(Default::default);
        c.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("PASETO v4.public")
                    .build(),
            ),
        );
        c.add_security_scheme(
            "carrier",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "x-carrier-signature",
                "HMAC-SHA256 over the request, with `x-carrier-id` and `x-carrier-timestamp` (PILOT-3)",
            ))),
        );
        c.add_security_scheme(
            "upstream",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "x-upstream-signature",
                "`sha256=<hex>` HMAC-SHA256 of the body, keyed by the peer's webhook secret",
            ))),
        );
        let bearer = || SecurityRequirement::new("bearer", Vec::<String>::new());
        for (path, item) in openapi.paths.paths.iter_mut() {
            let rule: Vec<SecurityRequirement> = if path == "/api/health" {
                Vec::new()
            } else if path.starts_with("/api/upstream/") {
                vec![SecurityRequirement::new("upstream", Vec::<String>::new())]
            } else if path.starts_with("/api/write-back/") {
                vec![
                    bearer(),
                    SecurityRequirement::new("carrier", Vec::<String>::new()),
                ]
            } else {
                vec![bearer()]
            };
            for (method, op) in [
                ("get", &mut item.get),
                ("put", &mut item.put),
                ("post", &mut item.post),
                ("delete", &mut item.delete),
                ("patch", &mut item.patch),
            ] {
                let Some(op) = op else { continue };
                op.security = Some(rule.clone());
                // utoipa names an operation after its function, and function names repeat across modules;
                // an `operationId` must be unique, and generated clients are named after it.
                let slug: String = path
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect();
                op.operation_id = Some(format!("{method}{}", slug.trim_end_matches('_')));
            }
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    modifiers(&Credentials),
    info(
        title = "Patient Administration System",
        description = "Administrative (not clinical) API for appointments, waiting lists, reminders and replies, transport and virtual-visit requests, analytics and tolerances. Every REST response is an `ApiResponse` envelope. Identity and workforce are held by person-service and worker-service; /api/patients and /api/practitioners proxy them. Request bodies and query parameters of the REST routes are described (QUAL-34); typed responses are not yet."
    ),
    paths(
        crate::api::rest::handlers::health,
        crate::api::rest::upstream_proxy::create_patient,
        crate::api::rest::upstream_proxy::search_patients,
        crate::api::rest::upstream_proxy::get_patient,
        crate::api::rest::upstream_proxy::update_patient,
        crate::api::rest::upstream_proxy::delete_patient,
        crate::api::rest::upstream_proxy::masked_patient,
        crate::api::rest::upstream_proxy::export_patient,
        crate::api::rest::upstream_proxy::audit_patient,
        crate::api::rest::upstream_proxy::export_patients_json,
        crate::api::rest::upstream_proxy::export_patients_xml,
        crate::api::rest::upstream_proxy::export_patients_tsv,
        crate::api::rest::upstream_proxy::export_patients_csv,
        crate::api::rest::upstream_proxy::import_patients,
        crate::api::rest::upstream_proxy::create_practitioner,
        crate::api::rest::upstream_proxy::search_practitioners,
        crate::api::rest::upstream_proxy::get_practitioner,
        crate::api::rest::upstream_proxy::update_practitioner,
        crate::api::rest::upstream_proxy::delete_practitioner,
        crate::api::rest::upstream_proxy::masked_practitioner,
        crate::api::rest::upstream_proxy::export_practitioner,
        crate::api::rest::upstream_proxy::audit_practitioner,
        crate::api::rest::upstream_proxy::person_merged_webhook,
        crate::api::rest::upstream_proxy::worker_merged_webhook,
        crate::api::rest::handlers_adt::admit,
        crate::api::rest::handlers_adt::transfer,
        crate::api::rest::handlers_adt::discharge,
        crate::api::rest::handlers_adt::cancel_admit,
        crate::api::rest::handlers_adt::cancel_discharge,
        crate::api::rest::handlers_scheduling::book_slot,
        crate::api::rest::handlers_contact::get_profile,
        crate::api::rest::handlers_contact::put_profile,
        crate::api::rest::handlers_contact::verify_mobile,
        crate::api::rest::handlers_contact::list_optouts,
        crate::api::rest::handlers_contact::add_optout,
        crate::api::rest::handlers_contact::remove_optout,
        crate::api::rest::handlers_contact::stale_profiles,
        crate::api::rest::handlers_standby::add_standby,
        crate::api::rest::handlers_standby::list_standby,
        crate::api::rest::handlers_standby::remove_standby,
        crate::api::rest::handlers_standby::book_from_standby,
        crate::api::rest::handlers_standby::list_releases,
        crate::api::rest::handlers_standby::release_candidates,
        crate::api::rest::handlers_writeback::post_reply,
        crate::api::rest::handlers_writeback::post_batch,
        crate::api::rest::handlers_links::get_link,
        crate::api::rest::handlers_links::post_link_rebook,
        crate::api::rest::handlers_links::post_link_action,
        crate::api::rest::handlers_links::open_calls,
        crate::api::rest::handlers_links::call_outcome,
        crate::api::rest::handlers_letters::create_group,
        crate::api::rest::handlers_letters::check_sms,
        crate::api::rest::handlers_writeback::post_text,
        crate::api::rest::handlers_writeback::open_texts,
        crate::api::rest::handlers_writeback::text_handled,
        crate::api::rest::handlers_scheduling::list_appointments,
        crate::api::rest::handlers_scheduling::list_patient_appointments,
        crate::api::rest::handlers_scheduling::cancel_appointment,
        crate::api::rest::handlers_scheduling::rebook_options,
        crate::api::rest::handlers_scheduling::rebook_appointment,
        crate::api::rest::handlers_history::appointment_history,
        crate::api::rest::handlers_history::contact_log,
        crate::api::rest::handlers_validation::start,
        crate::api::rest::handlers_validation::list,
        crate::api::rest::handlers_validation::get,
        crate::api::rest::handlers_validation::items,
        crate::api::rest::handlers_validation::asked,
        crate::api::rest::handlers_validation::answer,
        crate::api::rest::handlers_validation::letter_sent,
        crate::api::rest::handlers_validation::decision,
        crate::api::rest::handlers_validation::reinstate,
        crate::api::rest::handlers_validation::removals_report,
        crate::api::rest::handlers_waits::lead_time,
        crate::api::rest::handlers_waits::patient_waits,
        crate::api::rest::handlers_waits::due_for_check,
        crate::api::rest::handlers_waits::ask,
        crate::api::rest::handlers_waits::answer,
        crate::api::rest::handlers_transport::set_eligible,
        crate::api::rest::handlers_transport::withdraw_eligible,
        crate::api::rest::handlers_transport::record_consent,
        crate::api::rest::handlers_transport::withdraw_consent,
        crate::api::rest::handlers_transport::booking,
        crate::api::rest::handlers_analytics::outcomes,
        crate::api::rest::handlers_analytics::refill_rates,
        crate::api::rest::handlers_analytics::load_deprivation_lookup,
        crate::api::rest::handlers_analytics::refresh_demographics,
        crate::api::rest::handlers_analytics::set_demographics,
        crate::api::rest::handlers_offers::get_offer,
        crate::api::rest::handlers_offers::post_offer_action,
        crate::api::rest::handlers_offers::release_offers,
        crate::api::rest::handlers_offers::offer_now,
        crate::api::rest::handlers_referrals::create,
        crate::api::rest::handlers_referrals::list,
        crate::api::rest::handlers_referrals::get,
        crate::api::rest::handlers_referrals::book_first_available,
        crate::api::rest::handlers_referrals::waitlist,
        crate::api::rest::handlers_referrals::close,
        crate::api::rest::handlers_referrals::triage,
        crate::api::rest::handlers_reports::clinic_activity,
        crate::api::rest::handlers_follow_up_delay::delay_report,
        crate::api::rest::handlers_follow_up_delay::set_target,
        crate::api::rest::handlers_needs_review::record_review,
        crate::api::rest::handlers_needs_review::due_review,
        crate::api::rest::handlers_clinical_priority::set_priority,
        crate::api::rest::handlers_clinical_priority::overdue,
        crate::api::rest::handlers_triage::record,
        crate::api::rest::handlers_triage::list,
        crate::api::rest::handlers_triage::first_contact,
        crate::api::rest::handlers_triage::waits,
        crate::api::rest::handlers_triage::queue,
        crate::api::rest::handlers_proxy::grant,
        crate::api::rest::handlers_proxy::list,
        crate::api::rest::handlers_proxy::revoke,
        crate::api::rest::handlers_proxy::appointments,
        crate::api::rest::handlers_proxy::cancel,
        crate::api::rest::handlers_proxy::rebook,
        crate::api::rest::handlers_pifu::open,
        crate::api::rest::handlers_pifu::request,
        crate::api::rest::handlers_pifu::requests,
        crate::api::rest::handlers_pifu::handle,
        crate::api::rest::handlers_pifu::due_reviews,
        crate::api::rest::handlers_pifu::review,
        crate::api::rest::handlers_pifu::uptake,
        crate::api::rest::handlers_adjustments::list,
        crate::api::rest::handlers_adjustments::record,
        crate::api::rest::handlers_adjustments::codes,
        crate::api::rest::handlers_adjustments::for_appointments,
        crate::api::rest::handlers_adjustments::end,
        crate::api::fhir::handlers::search_flags,
        crate::api::fhir::handlers::search_observations,
        crate::api::fhir::handlers::search_subscription_topics,
        crate::api::fhir::handlers::search_subscriptions,
        crate::api::rest::handlers_subscriptions::create,
        crate::api::rest::handlers_subscriptions::list,
        crate::api::rest::handlers_subscriptions::end,
        crate::api::fhir::r4::get_appointment,
        crate::api::fhir::r4::get_encounter,
        crate::api::fhir::r4::get_service_request,
        crate::api::fhir::ihe_scheduling::find,
        crate::api::fhir::ihe_scheduling::hold,
        crate::api::fhir::ihe_scheduling::book,
        crate::api::fhir::ihe_scheduling::metadata,
        crate::api::rest::handlers_interpreter::set_need,
        crate::api::rest::handlers_interpreter::confirm,
        crate::api::rest::handlers_interpreter::unconfirmed,
        crate::api::rest::handlers_group_sessions::set_capacity,
        crate::api::rest::handlers_group_sessions::attendees,
        crate::api::rest::handlers_clinic_templates::ring_fence,
        crate::api::rest::handlers_clinic_templates::create,
        crate::api::rest::handlers_clinic_templates::list,
        crate::api::rest::handlers_clinic_templates::propose,
        crate::api::rest::handlers_clinic_templates::approve,
        crate::api::rest::handlers_clinic_templates::generate,
        crate::api::rest::handlers_clinic_templates::report,
        crate::api::rest::handlers_print::queue,
        crate::api::rest::handlers_print::export,
        crate::api::rest::handlers_print::mark_sent,
        crate::api::rest::handlers_contact::validate_nhs_number,
        crate::api::rest::handlers_receipts::post_receipt,
        crate::api::rest::handlers_receipts::letter_receipts,
        crate::api::rest::handlers_receipts::contact_problems,
        crate::api::rest::handlers_receipts::resolve_problem,
        crate::api::rest::handlers_identity::record,
        crate::api::rest::handlers_identity::get,
        crate::api::rest::handlers_identity::revoke,
        crate::api::rest::handlers_identity::check,
        crate::api::rest::handlers_experiments::start,
        crate::api::rest::handlers_experiments::list,
        crate::api::rest::handlers_experiments::stop,
        crate::api::rest::handlers_experiments::readout,
        crate::api::rest::handlers_tolerance::list,
        crate::api::rest::handlers_tolerance::status,
        crate::api::rest::handlers_tolerance::evaluate,
        crate::api::rest::handlers_tolerance::alerts,
        crate::api::rest::handlers_tolerance::acknowledge,
        crate::api::rest::handlers_tolerance::set,
        crate::api::rest::handlers_tolerance::reset,
        crate::api::rest::handlers_noshow::status,
        crate::api::rest::handlers_noshow::rollout,
        crate::api::rest::handlers_noshow::set_stage,
        crate::api::rest::handlers_noshow::evaluation,
        crate::api::rest::handlers_noshow::model_card,
        crate::api::rest::handlers_noshow::record_review,
        crate::api::rest::handlers_noshow::fairness_audit,
        crate::api::rest::handlers_noshow::recalculate,
        crate::api::rest::handlers_noshow::for_appointments,
        crate::api::rest::handlers_noshow::priority_list,
        crate::api::rest::handlers_noshow::for_appointment,
        crate::api::rest::handlers_whatsapp::record_optin,
        crate::api::rest::handlers_whatsapp::withdraw_optin,
        crate::api::rest::handlers_whatsapp::approve_template,
        crate::api::rest::handlers_whatsapp::revoke_template,
        crate::api::rest::handlers_ivr::menu,
        crate::api::rest::handlers_ivr::press,
        crate::api::rest::handlers_support::add,
        crate::api::rest::handlers_support::open,
        crate::api::rest::handlers_support::resolve,
        crate::api::rest::handlers_followup::policies,
        crate::api::rest::handlers_followup::approve_policy,
        crate::api::rest::handlers_followup::open_tasks,
        crate::api::rest::handlers_followup::close_task,
        crate::api::rest::handlers_virtual::list_services,
        crate::api::rest::handlers_virtual::approve_service,
        crate::api::rest::handlers_virtual::remove_service,
        crate::api::rest::handlers_virtual::pending,
        crate::api::rest::handlers_virtual::video_failed,
        crate::api::rest::handlers_virtual::review,
        crate::api::rest::handlers_scheduling::move_to_room,
        crate::api::rest::handlers_scheduling::in_facility,
        crate::api::rest::handlers_scheduling::record_barrier,
        crate::api::rest::handlers_scheduling::check_in_appointment,
        crate::api::rest::handlers_scheduling::complete_appointment,
        crate::api::rest::handlers_resources::ward_occupancy,
        crate::api::rest::handlers_resources::list_beds_by_ward,
        crate::api::rest::handlers_resources::get_bed,
        crate::api::rest::handlers_resources::set_bed_status,
        crate::api::rest::handlers_adt::pre_admit,
        crate::api::rest::handlers_adt::cancel_pre_admit,
        crate::api::rest::handlers_adt::change_to_inpatient,
        crate::api::rest::handlers_adt::change_to_outpatient,
        crate::api::rest::handlers_adt::cancel_transfer,
        crate::api::rest::handlers_adt::leave_start,
        crate::api::rest::handlers_adt::leave_end,
        crate::api::rest::handlers_scheduling::mark_no_show,
        crate::api::rest::handlers_series::create_series,
        crate::api::rest::handlers_series::get_series,
        crate::api::rest::handlers_series::cancel_series,
        crate::api::rest::handlers_series::list_series_for_person,
        crate::api::rest::handlers_encounter::get_encounter,
        crate::api::rest::handlers_encounter::set_status,
        crate::api::rest::handlers_encounter::cancel_encounter,
        crate::api::rest::handlers_encounter::list_encounters_for_person,
        crate::api::rest::handlers_billing::open_account,
        crate::api::rest::handlers_billing::close_account,
        crate::api::rest::handlers_billing::post_charge,
        crate::api::rest::handlers_billing::finalize_invoice,
        crate::api::rest::handlers_billing::post_payment,
        crate::api::rest::handlers_coverage::create_coverage,
        crate::api::rest::handlers_coverage::get_coverage,
        crate::api::rest::handlers_coverage::update_coverage,
        crate::api::rest::handlers_coverage::delete_coverage,
        crate::api::rest::handlers_coverage::list_coverages_for_person,
        crate::api::rest::handlers_coverage::list_coverages_for_account,
        crate::api::rest::handlers_consent::create_consent,
        crate::api::rest::handlers_consent::list_consents_for_person,
        crate::api::rest::handlers_consent::revoke_consent,
        crate::api::rest::handlers_waitlist::add_to_waitlist,
        crate::api::rest::handlers_waitlist::list_waitlist,
        crate::api::rest::handlers_waitlist::remove_from_waitlist,
        crate::api::rest::handlers_waitlist::book_from_waitlist,
        crate::api::rest::handlers_waitlist::rtt_start,
        crate::api::rest::handlers_waitlist::rtt_pause,
        crate::api::rest::handlers_waitlist::rtt_resume,
        crate::api::rest::handlers_waitlist::rtt_stop,
        crate::api::rest::handlers_waitlist::rtt_weeks_waiting,
        crate::api::rest::handlers_waitlist::rtt_is_breaching,
        crate::api::rest::handlers_audit::list_recent,
        crate::api::rest::handlers_audit::list_for_entity,
        crate::api::rest::handlers_audit::export_balp,
        crate::api::rest::handlers_audit::list_local_for_person,
        crate::api::rest::handlers_admin::metrics,
        crate::api::rest::handlers_admin::list_unpublished_outbox,
        crate::api::rest::handlers_admin::list_dead_letters,
        crate::api::rest::handlers_admin::replay_dead_letter,
        crate::api::rest::handlers_resources::create_facility,
        crate::api::rest::handlers_resources::list_facilities,
        crate::api::rest::handlers_resources::get_facility,
        crate::api::rest::handlers_resources::list_wards_by_facility,
        crate::api::rest::handlers_resources::create_ward,
        crate::api::rest::handlers_resources::get_ward,
        crate::api::rest::handlers_resources::list_rooms_by_ward,
        crate::api::rest::handlers_resources::create_room,
        crate::api::rest::handlers_resources::get_room,
        crate::api::rest::handlers_resources::create_bed,
        crate::api::rest::handlers_letters::create_template,
        crate::api::rest::handlers_letters::list_templates,
        crate::api::rest::handlers_letters::get_template,
        crate::api::rest::handlers_letters::update_template,
        crate::api::rest::handlers_letters::preview_letter,
        crate::api::rest::handlers_letters::generate_letter,
        crate::api::rest::handlers_letters::list_patient_letters,
        crate::api::rest::handlers_letters::get_letter,
        crate::api::rest::handlers_letters::mark_sent,
        crate::api::rest::handlers_letters::mark_failed,
        crate::api::rest::handlers_hl7v2::parse,
        crate::api::rest::handlers_hl7v2::add_person,
        crate::api::rest::handlers_hl7v2::admit,
        crate::api::rest::handlers_hl7v2::transfer,
        crate::api::rest::handlers_hl7v2::discharge,
        crate::api::rest::handlers_hl7v2::register,
        crate::api::rest::handlers_hl7v2::pre_admit,
        crate::api::rest::handlers_hl7v2::cancel_pre_admit,
        crate::api::rest::handlers_hl7v2::change_to_inpatient,
        crate::api::rest::handlers_hl7v2::change_to_outpatient,
        crate::api::rest::handlers_hl7v2::update,
        crate::api::rest::handlers_hl7v2::cancel_admit,
        crate::api::rest::handlers_hl7v2::cancel_transfer,
        crate::api::rest::handlers_hl7v2::cancel_discharge,
        crate::api::rest::handlers_hl7v2::leave_start,
        crate::api::rest::handlers_hl7v2::leave_end,
        crate::api::rest::handlers_hl7v2::delete_patient,
        crate::api::rest::handlers_hl7v2::merge,
        crate::api::rest::handlers_hl7v2::dft,
        crate::api::rest::handlers_hl7v2::schedule_book,
        crate::api::rest::handlers_hl7v2::schedule_reschedule,
        crate::api::rest::handlers_hl7v2::schedule_modify,
        crate::api::rest::handlers_hl7v2::schedule_cancel,
        crate::api::rest::handlers_hl7v2::mfn_staff,
        crate::api::rest::handlers_hl7v2::mfn_location,
        crate::api::rest::handlers_hl7v2::batch,
        crate::api::fhir::handlers::post_bundle,
        crate::api::fhir::handlers::metadata,
        crate::api::fhir::handlers::create_patient,
        crate::api::fhir::handlers::list_patients,
        crate::api::fhir::handlers::get_patient,
        crate::api::fhir::handlers::update_patient,
        crate::api::fhir::handlers::delete_patient,
        crate::api::fhir::handlers::get_encounter,
        crate::api::fhir::handlers::get_appointment,
        crate::api::fhir::handlers::create_practitioner,
        crate::api::fhir::handlers::get_practitioner,
        crate::api::fhir::handlers::update_practitioner,
        crate::api::fhir::handlers::delete_practitioner,
        crate::api::fhir::handlers::create_schedule,
        crate::api::fhir::handlers::get_schedule,
        crate::api::fhir::handlers::update_schedule,
        crate::api::fhir::handlers::delete_schedule,
        crate::api::fhir::handlers::create_slot,
        crate::api::fhir::handlers::get_slot,
        crate::api::fhir::handlers::update_slot,
        crate::api::fhir::handlers::delete_slot,
        crate::api::fhir::handlers::get_location,
        crate::api::fhir::handlers::get_coverage,
        alias_api_patients_search_get,
        alias_api_practitioners_search_get,
    ),
    components(schemas(
        FhirBundleRequest,
        FhirBundleResponse,
        FhirBundleWriteEntry,
        FhirWriteBundle,
    ))
)]
pub struct ApiDoc;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every `(path, method)` registered with `.route(` in a routing source file.
    fn routed(src: &str) -> BTreeSet<(String, String)> {
        let mut out = BTreeSet::new();
        let mut rest = src;
        while let Some(at) = rest.find(".route(") {
            let body = &rest[at + ".route(".len()..];
            let mut depth = 1usize;
            let mut end = 0;
            for (i, c) in body.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let call = &body[..end];
            let open = call.find('"').expect("a route path") + 1;
            let close = open + call[open..].find('"').expect("a closing quote");
            let path = call[open..close].to_string();
            let tail = &call[close..];
            for method in ["get", "post", "put", "delete", "patch"] {
                let pat = format!("{method}(");
                let mut from = 0;
                while let Some(i) = tail[from..].find(&pat) {
                    let abs = from + i;
                    let before = tail[..abs].chars().next_back();
                    if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        out.insert((path.clone(), method.to_string()));
                    }
                    from = abs + pat.len();
                }
            }
            rest = &body[end..];
        }
        out
    }

    fn documented() -> BTreeSet<(String, String)> {
        let doc = serde_json::to_value(ApiDoc::openapi()).expect("serialize");
        let mut out = BTreeSet::new();
        for (path, item) in doc["paths"].as_object().expect("paths") {
            for method in item.as_object().expect("path item").keys() {
                out.insert((path.clone(), method.clone()));
            }
        }
        out
    }

    #[test]
    fn every_routed_path_and_method_is_documented() {
        let mut routes = std::collections::BTreeSet::new();
        for src in [
            include_str!("rest/routes/mod.rs"),
            include_str!("rest/routes/directory.rs"),
            include_str!("rest/routes/clinical.rs"),
            include_str!("rest/routes/messaging_waiting.rs"),
            include_str!("rest/routes/reports_records.rs"),
            include_str!("rest/routes/hl7.rs"),
            include_str!("fhir/routes.rs"),
        ] {
            routes.extend(routed(src));
        }
        assert!(routes.len() > 250, "the route scan found {}", routes.len());
        let docs = documented();
        let missing: Vec<_> = routes.difference(&docs).collect();
        assert!(
            missing.is_empty(),
            "routes with no #[utoipa::path] (add one and list it in ApiDoc): {missing:?}"
        );
        let extra: Vec<_> = docs.difference(&routes).collect();
        assert!(extra.is_empty(), "documented but not routed: {extra:?}");
    }

    /// A REST handler that takes a typed JSON body must say so in its `#[utoipa::path]`. A body taken as
    /// `serde_json::Value` is free-form and may stay undescribed.
    #[test]
    fn a_handler_with_a_typed_json_body_documents_it() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/rest");
        let mut missing = Vec::new();
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect("src/api/rest") {
            let path = entry.expect("entry").path();
            if path.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("read");
            for block in src.split("#[utoipa::path(").skip(1) {
                let Some((attr, rest)) = block.split_once("\n)]\n") else {
                    continue;
                };
                let Some(sig) = rest.split(") ->").next() else {
                    continue;
                };
                let Some(at) = sig.find("Json<") else {
                    continue;
                };
                let ty: String = sig[at + 5..].chars().take_while(|c| *c != '>').collect();
                if ty == "Value" || ty == "serde_json::Value" {
                    continue;
                }
                seen += 1;
                if !attr.contains("request_body") {
                    let name = rest
                        .split("fn ")
                        .nth(1)
                        .and_then(|f| f.split('(').next())
                        .unwrap_or("?");
                    missing.push(format!("{}::{name} ({ty})", path.display()));
                }
            }
        }
        assert!(
            seen > 80,
            "the scan found only {seen} typed bodies: it is broken"
        );
        assert!(
            missing.is_empty(),
            "typed JSON bodies not described in the OpenAPI document: {missing:#?}"
        );
    }

    /// A REST handler that takes `Query<T>` must list `T` among the `params(...)` of its `#[utoipa::path]`,
    /// so the document names every query parameter.
    #[test]
    fn a_handler_with_a_typed_query_documents_its_parameters() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/rest");
        let mut missing = Vec::new();
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect("src/api/rest") {
            let path = entry.expect("entry").path();
            if path.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("read");
            for block in src.split("#[utoipa::path(").skip(1) {
                let Some((attr, rest)) = block.split_once("\n)]\n") else {
                    continue;
                };
                let Some(sig) = rest.split(") ->").next() else {
                    continue;
                };
                let Some(at) = sig.find("Query<") else {
                    continue;
                };
                let ty: String = sig[at + 6..].chars().take_while(|c| *c != '>').collect();
                let name = ty.rsplit("::").next().unwrap_or(&ty);
                seen += 1;
                let params = attr.split("params(").nth(1).unwrap_or("");
                if !params.contains(name) {
                    let f = rest
                        .split("fn ")
                        .nth(1)
                        .and_then(|f| f.split('(').next())
                        .unwrap_or("?");
                    missing.push(format!("{}::{f} ({ty})", path.display()));
                }
            }
        }
        assert!(
            seen > 40,
            "the scan found only {seen} typed queries: it is broken"
        );
        assert!(
            missing.is_empty(),
            "query parameters not described in the OpenAPI document: {missing:#?}"
        );
    }

    #[test]
    fn every_route_says_what_credential_it_needs() {
        let doc = serde_json::to_value(ApiDoc::openapi()).expect("serialize");
        let paths = doc["paths"].as_object().expect("paths");
        for (path, item) in paths {
            for (method, op) in item.as_object().expect("item") {
                let security = &op["security"];
                assert!(security.is_array(), "{method} {path} has no security rule");
                let open = security.as_array().expect("array").is_empty();
                assert_eq!(open, path == "/api/health", "{method} {path}");
            }
        }
        assert!(doc["components"]["securitySchemes"]["bearer"].is_object());
    }

    #[test]
    fn every_operation_has_its_own_id() {
        let doc = serde_json::to_value(ApiDoc::openapi()).expect("serialize");
        let mut seen = std::collections::HashMap::new();
        for (path, item) in doc["paths"].as_object().expect("paths") {
            for (method, op) in item.as_object().expect("item") {
                let id = op["operationId"].as_str().expect("operationId");
                if let Some(other) = seen.insert(id.to_string(), format!("{method} {path}")) {
                    panic!("{id} names both {other} and {method} {path}");
                }
            }
        }
    }

    #[test]
    fn the_committed_document_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../spec/openapi.json");
        let mut now = serde_json::to_value(ApiDoc::openapi()).expect("serialize");
        let text = serde_json::to_string_pretty(&now).expect("pretty") + "\n";
        if std::env::var("UPDATE_OPENAPI").is_ok() {
            std::fs::write(path, text).expect("write spec/openapi.json");
            return;
        }
        let mut committed: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(path).expect("spec/openapi.json (UPDATE_OPENAPI=1 writes it)"),
        )
        .expect("valid JSON");
        // A version bump alone is not drift.
        now["info"]["version"] = "".into();
        committed["info"]["version"] = "".into();
        assert!(
            now == committed,
            "spec/openapi.json is stale: run `UPDATE_OPENAPI=1 cargo test -p patient-administration-system openapi`"
        );
    }
}
