//! Every documented endpoint answers hostile, empty input with a client error or a plain answer, never a server
//! error (NFR-1): each is called with a random identifier and an empty body. A broad safety net that also
//! exercises each handler's refusal path. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;
use uuid::Uuid;

/// Answers that are not faults of the server: an upstream that is not configured (503) or a feature that is
/// not switched on (501).
fn acceptable(status: StatusCode) -> bool {
    !status.is_server_error()
        || status == StatusCode::SERVICE_UNAVAILABLE
        || status == StatusCode::NOT_IMPLEMENTED
}

const ENDPOINTS: &[(&str, &str)] = &[
    ("DELETE", "/api/admin/subscriptions/{id}"),
    ("DELETE", "/api/coverages/{id}"),
    ("DELETE", "/api/identity-proofings/{subject}"),
    ("DELETE", "/api/letter-templates/{id}/whatsapp-approval"),
    ("DELETE", "/api/patients/{id}"),
    ("DELETE", "/api/patients/{id}/channel-optouts/{channel}"),
    ("DELETE", "/api/patients/{id}/transport-consent"),
    ("DELETE", "/api/patients/{id}/transport-eligibility"),
    ("DELETE", "/api/patients/{id}/whatsapp-optin"),
    ("DELETE", "/api/practitioners/{id}"),
    ("DELETE", "/api/standby/{id}"),
    ("DELETE", "/api/tolerances/{metric}"),
    ("DELETE", "/api/virtual-ok-services/{service_type}"),
    ("DELETE", "/api/waitlist/{id}"),
    ("DELETE", "/fhir/Patient/{id}"),
    ("DELETE", "/fhir/Practitioner/{id}"),
    ("DELETE", "/fhir/Schedule/{id}"),
    ("DELETE", "/fhir/Slot/{id}"),
    ("GET", "/api/accounts/{id}/coverages"),
    ("GET", "/api/admin/metrics"),
    ("GET", "/api/admin/outbox/dead-letters"),
    ("GET", "/api/admin/outbox/unpublished"),
    ("GET", "/api/admin/subscriptions"),
    ("GET", "/api/admissions/{id}"),
    ("GET", "/api/analytics/appointment-outcomes"),
    ("GET", "/api/analytics/refill-rates"),
    ("GET", "/api/appointment-series/{id}"),
    ("GET", "/api/appointments"),
    ("GET", "/api/appointments/{id}"),
    ("GET", "/api/appointments/{id}/history"),
    ("GET", "/api/appointments/{id}/lead-time"),
    ("GET", "/api/appointments/{id}/noshow"),
    ("GET", "/api/appointments/{id}/rebook-options"),
    ("GET", "/api/appointments/{id}/transport"),
    ("GET", "/api/audit/balp"),
    ("GET", "/api/audit/entity"),
    ("GET", "/api/audit/recent"),
    ("GET", "/api/beds/{id}"),
    ("GET", "/api/beds/{id}/occupancy-events"),
    ("GET", "/api/capacity/report"),
    ("GET", "/api/clinical-priority-reviews/overdue"),
    ("GET", "/api/communication-needs/due-review"),
    ("GET", "/api/communication-profiles/stale"),
    ("GET", "/api/contact-problems"),
    ("GET", "/api/conversions/pending"),
    ("GET", "/api/coverages/{id}"),
    ("GET", "/api/encounters/{id}"),
    ("GET", "/api/encounters/{id}/triage"),
    ("GET", "/api/experiments"),
    ("GET", "/api/experiments/{id}/readout"),
    ("GET", "/api/facilities"),
    ("GET", "/api/facilities/{id}"),
    ("GET", "/api/facilities/{id}/wards"),
    ("GET", "/api/facility/in-facility"),
    ("GET", "/api/follow-up-policies"),
    ("GET", "/api/follow-up-tasks/open"),
    ("GET", "/api/health"),
    ("GET", "/api/identifier-schemes"),
    ("GET", "/api/identity-proofings/{subject}"),
    ("GET", "/api/identity-proofings/{subject}/check"),
    ("GET", "/api/inbound-texts/open"),
    ("GET", "/api/interpreter-requests/unconfirmed"),
    ("GET", "/api/letter-templates"),
    ("GET", "/api/letter-templates/{id}"),
    ("GET", "/api/letters/{id}"),
    ("GET", "/api/letters/{id}/receipts"),
    ("GET", "/api/noshow/evaluation"),
    ("GET", "/api/noshow/fairness-audit"),
    ("GET", "/api/noshow/for-appointments"),
    ("GET", "/api/noshow/model-card"),
    ("GET", "/api/noshow/priority-list"),
    ("GET", "/api/noshow/rollout"),
    ("GET", "/api/noshow/status"),
    ("GET", "/api/offer-links/{token}"),
    ("GET", "/api/pathways/due-check"),
    ("GET", "/api/patients"),
    ("GET", "/api/patients/export.csv"),
    ("GET", "/api/patients/export.json"),
    ("GET", "/api/patients/export.tsv"),
    ("GET", "/api/patients/export.xml"),
    ("GET", "/api/patients/search"),
    ("GET", "/api/patients/{id}"),
    ("GET", "/api/patients/{id}/appointment-series"),
    ("GET", "/api/patients/{id}/appointments"),
    ("GET", "/api/patients/{id}/audit"),
    ("GET", "/api/patients/{id}/audit/local"),
    ("GET", "/api/patients/{id}/channel-optouts"),
    ("GET", "/api/patients/{id}/communication-profile"),
    ("GET", "/api/patients/{id}/consents"),
    ("GET", "/api/patients/{id}/contact-log"),
    ("GET", "/api/patients/{id}/coverages"),
    ("GET", "/api/patients/{id}/encounters"),
    ("GET", "/api/patients/{id}/enquiry"),
    ("GET", "/api/patients/{id}/export"),
    ("GET", "/api/patients/{id}/letters"),
    ("GET", "/api/patients/{id}/masked"),
    ("GET", "/api/patients/{id}/proxies"),
    ("GET", "/api/patients/{id}/reasonable-adjustments"),
    ("GET", "/api/patients/{id}/waits"),
    ("GET", "/api/pifu/due-reviews"),
    ("GET", "/api/pifu/requests"),
    ("GET", "/api/practitioners"),
    ("GET", "/api/practitioners/search"),
    ("GET", "/api/practitioners/{id}"),
    ("GET", "/api/practitioners/{id}/audit"),
    ("GET", "/api/practitioners/{id}/export"),
    ("GET", "/api/practitioners/{id}/masked"),
    ("GET", "/api/print-queue"),
    ("GET", "/api/proxy/patients/{id}/appointments"),
    ("GET", "/api/reasonable-adjustments/codes"),
    ("GET", "/api/reasonable-adjustments/for-appointments"),
    ("GET", "/api/referrals"),
    ("GET", "/api/referrals/{id}"),
    ("GET", "/api/referrals/{id}/suspicions"),
    ("GET", "/api/reply-links/{token}"),
    ("GET", "/api/reports/clinic-activity"),
    ("GET", "/api/reports/follow-up-delay"),
    ("GET", "/api/reports/pifu"),
    ("GET", "/api/reports/validation-removals"),
    ("GET", "/api/rooms/{id}"),
    ("GET", "/api/rtt/{id}"),
    ("GET", "/api/rtt/{id}/breaching"),
    ("GET", "/api/rtt/{id}/weeks-waiting"),
    ("GET", "/api/session-templates"),
    ("GET", "/api/slot-releases"),
    ("GET", "/api/slot-releases/{id}/candidates"),
    ("GET", "/api/slot-releases/{id}/offers"),
    ("GET", "/api/slots/{id}"),
    ("GET", "/api/slots/{id}/attendees"),
    ("GET", "/api/staff-call-tasks/open"),
    ("GET", "/api/standby"),
    ("GET", "/api/standby/{id}"),
    ("GET", "/api/support-flags/open"),
    ("GET", "/api/suspicion-profile"),
    ("GET", "/api/suspicions"),
    ("GET", "/api/suspicions/report"),
    ("GET", "/api/suspicions/{id}"),
    ("GET", "/api/tolerance-alerts"),
    ("GET", "/api/tolerances"),
    ("GET", "/api/tolerances/status"),
    ("GET", "/api/triage/queue"),
    ("GET", "/api/triage/waits"),
    ("GET", "/api/validation-campaigns"),
    ("GET", "/api/validation-campaigns/{id}"),
    ("GET", "/api/validation-campaigns/{id}/items"),
    ("GET", "/api/virtual-ok-services"),
    ("GET", "/api/waitlist"),
    ("GET", "/api/waitlist/{id}"),
    ("GET", "/api/wards/{id}"),
    ("GET", "/api/wards/{id}/beds"),
    ("GET", "/api/wards/{id}/occupancy"),
    ("GET", "/api/wards/{id}/overview"),
    ("GET", "/api/wards/{id}/rooms"),
    ("GET", "/api/work-items"),
    ("GET", "/api/work-items/reminders"),
    ("GET", "/api/work-items/{id}"),
    ("GET", "/fhir/Appointment/{id}"),
    ("GET", "/fhir/Coverage/{id}"),
    ("GET", "/fhir/Encounter/{id}"),
    ("GET", "/fhir/Flag"),
    ("GET", "/fhir/Location/{id}"),
    ("GET", "/fhir/Observation"),
    ("GET", "/fhir/Patient"),
    ("GET", "/fhir/Patient/{id}"),
    ("GET", "/fhir/Practitioner/{id}"),
    ("GET", "/fhir/Schedule/{id}"),
    ("GET", "/fhir/Slot/{id}"),
    ("GET", "/fhir/Subscription"),
    ("GET", "/fhir/SubscriptionTopic"),
    ("GET", "/fhir/metadata"),
    ("GET", "/fhir/r4/metadata"),
    ("POST", "/api/accounts"),
    ("POST", "/api/accounts/{id}/close"),
    ("POST", "/api/admin/outbox/dead-letters/{id}/replay"),
    ("POST", "/api/admin/outbox/dead-letters/{id}/skip"),
    ("POST", "/api/admin/subscriptions"),
    ("POST", "/api/admissions"),
    ("POST", "/api/admissions/cancel-pre-admit"),
    ("POST", "/api/admissions/change-to-inpatient"),
    ("POST", "/api/admissions/pre-admit"),
    ("POST", "/api/admissions/{id}/cancel-admit"),
    ("POST", "/api/admissions/{id}/cancel-discharge"),
    ("POST", "/api/admissions/{id}/cancel-transfer"),
    ("POST", "/api/admissions/{id}/change-to-outpatient"),
    ("POST", "/api/admissions/{id}/discharge"),
    ("POST", "/api/admissions/{id}/leave-end"),
    ("POST", "/api/admissions/{id}/leave-start"),
    ("POST", "/api/admissions/{id}/transfer"),
    ("POST", "/api/analytics/demographics/refresh"),
    ("POST", "/api/appointment-series"),
    ("POST", "/api/appointment-series/{id}/cancel"),
    ("POST", "/api/appointments/{id}/cancel"),
    ("POST", "/api/appointments/{id}/check-in"),
    ("POST", "/api/appointments/{id}/complete"),
    ("POST", "/api/appointments/{id}/conversion/{decision}"),
    ("POST", "/api/appointments/{id}/no-show"),
    ("POST", "/api/appointments/{id}/rebook"),
    ("POST", "/api/appointments/{id}/video-failed"),
    ("POST", "/api/beds"),
    ("POST", "/api/charges"),
    ("POST", "/api/consents/{id}/revoke"),
    ("POST", "/api/contact-problems/{person_id}/resolve"),
    ("POST", "/api/coverages"),
    ("POST", "/api/encounters/{id}/cancel"),
    ("POST", "/api/encounters/{id}/first-contact"),
    ("POST", "/api/encounters/{id}/triage"),
    ("POST", "/api/experiments"),
    ("POST", "/api/experiments/{id}/stop"),
    ("POST", "/api/facilities"),
    ("POST", "/api/follow-up-tasks/{id}/outcome"),
    ("POST", "/api/hl7/v2/admit"),
    ("POST", "/api/hl7/v2/batch"),
    ("POST", "/api/hl7/v2/cancel-admit"),
    ("POST", "/api/hl7/v2/cancel-discharge"),
    ("POST", "/api/hl7/v2/cancel-pre-admit"),
    ("POST", "/api/hl7/v2/cancel-transfer"),
    ("POST", "/api/hl7/v2/change-to-inpatient"),
    ("POST", "/api/hl7/v2/change-to-outpatient"),
    ("POST", "/api/hl7/v2/delete-patient"),
    ("POST", "/api/hl7/v2/dft"),
    ("POST", "/api/hl7/v2/discharge"),
    ("POST", "/api/hl7/v2/leave-end"),
    ("POST", "/api/hl7/v2/leave-start"),
    ("POST", "/api/hl7/v2/merge"),
    ("POST", "/api/hl7/v2/mfn-location"),
    ("POST", "/api/hl7/v2/mfn-staff"),
    ("POST", "/api/hl7/v2/parse"),
    ("POST", "/api/hl7/v2/patient"),
    ("POST", "/api/hl7/v2/pre-admit"),
    ("POST", "/api/hl7/v2/register"),
    ("POST", "/api/hl7/v2/schedule-book"),
    ("POST", "/api/hl7/v2/schedule-cancel"),
    ("POST", "/api/hl7/v2/schedule-modify"),
    ("POST", "/api/hl7/v2/schedule-reschedule"),
    ("POST", "/api/hl7/v2/transfer"),
    ("POST", "/api/hl7/v2/update"),
    ("POST", "/api/inbound-texts/{id}/handled"),
    ("POST", "/api/interpreter-requests/{id}/confirm"),
    ("POST", "/api/invoices"),
    ("POST", "/api/letter-template-groups"),
    ("POST", "/api/letter-templates"),
    ("POST", "/api/letter-templates/check-sms"),
    ("POST", "/api/letters/generate"),
    ("POST", "/api/letters/preview"),
    ("POST", "/api/letters/{id}/failed"),
    ("POST", "/api/letters/{id}/sent"),
    ("POST", "/api/noshow/recalculate"),
    ("POST", "/api/noshow/reviews"),
    ("POST", "/api/offer-links/{token}/{action}"),
    ("POST", "/api/pathways/{id}/still-needed-checks"),
    ("POST", "/api/patients"),
    ("POST", "/api/patients/import"),
    ("POST", "/api/patients/{id}/channel-optouts"),
    ("POST", "/api/patients/{id}/communication-needs/review"),
    (
        "POST",
        "/api/patients/{id}/communication-profile/verify-mobile",
    ),
    ("POST", "/api/patients/{id}/consents"),
    ("POST", "/api/patients/{id}/pifu"),
    ("POST", "/api/patients/{id}/proxies"),
    ("POST", "/api/patients/{id}/reasonable-adjustments"),
    ("POST", "/api/payments"),
    ("POST", "/api/pifu/requests/{id}/handle"),
    ("POST", "/api/pifu/{id}/request"),
    ("POST", "/api/pifu/{id}/review"),
    ("POST", "/api/practitioners"),
    ("POST", "/api/print-batches"),
    ("POST", "/api/print-batches/{id}/sent"),
    ("POST", "/api/proxies/{id}/revoke"),
    ("POST", "/api/proxy/appointments/{id}/cancel"),
    ("POST", "/api/proxy/appointments/{id}/rebook"),
    ("POST", "/api/reasonable-adjustments/{id}/end"),
    ("POST", "/api/referrals"),
    ("POST", "/api/referrals/{id}/book-first-available"),
    ("POST", "/api/referrals/{id}/close"),
    ("POST", "/api/referrals/{id}/suspicions"),
    ("POST", "/api/referrals/{id}/triage"),
    ("POST", "/api/referrals/{id}/waitlist"),
    ("POST", "/api/reply-links/{token}/rebook"),
    ("POST", "/api/reply-links/{token}/{action}"),
    ("POST", "/api/rooms"),
    ("POST", "/api/rtt/start"),
    ("POST", "/api/rtt/{id}/pause"),
    ("POST", "/api/rtt/{id}/resume"),
    ("POST", "/api/rtt/{id}/stop"),
    ("POST", "/api/session-templates"),
    ("POST", "/api/session-templates/{id}/generate"),
    ("POST", "/api/session-templates/{id}/versions"),
    (
        "POST",
        "/api/session-templates/{id}/versions/{version}/approve",
    ),
    ("POST", "/api/slot-releases/{id}/offer"),
    ("POST", "/api/slots/{id}/book"),
    ("POST", "/api/staff-call-tasks/{id}/outcome"),
    ("POST", "/api/standby"),
    ("POST", "/api/standby/{id}/book"),
    ("POST", "/api/still-needed-checks/{id}/answer"),
    ("POST", "/api/support-flags"),
    ("POST", "/api/support-flags/{id}/resolve"),
    ("POST", "/api/suspicions/{id}/amend"),
    ("POST", "/api/suspicions/{id}/stop"),
    ("POST", "/api/tolerance-alerts/{id}/acknowledge"),
    ("POST", "/api/tolerances/evaluate"),
    ("POST", "/api/upstream/person-merged"),
    ("POST", "/api/upstream/worker-merged"),
    ("POST", "/api/validate/identifier"),
    ("POST", "/api/validation-campaigns"),
    ("POST", "/api/validation-campaigns/{id}/items/{item}/answer"),
    ("POST", "/api/validation-campaigns/{id}/items/{item}/asked"),
    (
        "POST",
        "/api/validation-campaigns/{id}/items/{item}/decision",
    ),
    (
        "POST",
        "/api/validation-campaigns/{id}/items/{item}/letter-sent",
    ),
    (
        "POST",
        "/api/validation-campaigns/{id}/items/{item}/reinstate",
    ),
    ("POST", "/api/waitlist"),
    ("POST", "/api/waitlist/{id}/book"),
    ("POST", "/api/wards"),
    ("POST", "/api/work-items/{id}/assign"),
    ("POST", "/api/work-items/{id}/claim"),
    ("POST", "/api/work-items/{id}/release"),
    ("POST", "/api/work-queues/{queue}/next"),
    ("POST", "/api/write-back/batch"),
    ("POST", "/api/write-back/ivr/menu"),
    ("POST", "/api/write-back/ivr/press"),
    ("POST", "/api/write-back/receipts"),
    ("POST", "/api/write-back/replies"),
    ("POST", "/api/write-back/texts"),
    ("POST", "/fhir"),
    ("POST", "/fhir/Patient"),
    ("POST", "/fhir/Practitioner"),
    ("POST", "/fhir/Schedule"),
    ("POST", "/fhir/Slot"),
    ("POST", "/fhir/r4/Appointment/$book"),
    ("POST", "/fhir/r4/Appointment/$find"),
    ("POST", "/fhir/r4/Appointment/$hold"),
    ("PUT", "/api/analytics/demographics/{person_id}"),
    ("PUT", "/api/analytics/deprivation-lookup"),
    ("PUT", "/api/appointments/{id}/barrier"),
    ("PUT", "/api/appointments/{id}/location"),
    ("PUT", "/api/beds/{id}/status"),
    ("PUT", "/api/coverages/{id}"),
    ("PUT", "/api/encounters/{id}/status"),
    ("PUT", "/api/follow-up-policies/{service_type}"),
    ("PUT", "/api/identity-proofings/{subject}"),
    ("PUT", "/api/letter-templates/{id}"),
    ("PUT", "/api/letter-templates/{id}/whatsapp-approval"),
    ("PUT", "/api/noshow/rollout/{service_type}"),
    ("PUT", "/api/patients/{id}"),
    ("PUT", "/api/patients/{id}/communication-profile"),
    ("PUT", "/api/patients/{id}/interpreter-need"),
    ("PUT", "/api/patients/{id}/transport-consent"),
    ("PUT", "/api/patients/{id}/transport-eligibility"),
    ("PUT", "/api/patients/{id}/whatsapp-optin"),
    ("PUT", "/api/practitioners/{id}"),
    ("PUT", "/api/slots/{id}/capacity"),
    ("PUT", "/api/slots/{id}/ring-fence"),
    ("PUT", "/api/tolerances/{metric}"),
    ("PUT", "/api/virtual-ok-services/{service_type}"),
    ("PUT", "/api/waitlist/{id}/clinical-priority"),
    ("PUT", "/api/waitlist/{id}/follow-up-target"),
    ("PUT", "/fhir/Patient/{id}"),
    ("PUT", "/fhir/Practitioner/{id}"),
    ("PUT", "/fhir/Schedule/{id}"),
    ("PUT", "/fhir/Slot/{id}"),
];

/// What each kind of call is sent: queries that the list and report handlers read, and bodies that name the fields
/// handlers commonly require, so each refusal path and some success paths run, not only the first check.
const QUERIES: &[&str] = &[
    "",
    "?limit=1",
    "?limit=lots",
    "?cursor=bad",
    "?from=2030-01-01T00:00:00Z&to=2030-02-01T00:00:00Z",
    "?from=bad&to=worse",
    "?months=0",
    "?k=1",
    "?within_hours=0",
    "?q=a&status=open&service=x",
];

fn bodies(person: Uuid) -> Vec<String> {
    vec![
        "{}".to_string(),
        "[]".to_string(),
        serde_json::json!({
            "person_id": person, "bed_id": person, "slot_id": person, "worker_id": person,
            "recorded_by": "Receptionist", "reviewed_by": "Nurse", "set_by": "Clinician",
            "confirmed_by": "Coordinator", "reason": "because", "note": "a note", "code": "x",
            "language": "pl", "kind": "spoken", "status": "open", "channel": "post",
            "clinical_priority": "routine", "target_date": "2030-01-01",
            "review_by": "2030-01-01", "name": "n", "limit": 1,
        })
        .to_string(),
    ]
}

fn fill(template: &str) -> String {
    let mut uri = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let end = rest[start..]
            .find('}')
            .map_or(rest.len(), |i| start + i + 1);
        uri.push_str(&rest[..start]);
        let name = &rest[start + 1..end.saturating_sub(1).max(start + 1)];
        if name.ends_with("id") {
            uri.push_str(&Uuid::new_v4().to_string());
        } else {
            uri.push_str("unknown");
        }
        rest = &rest[end..];
    }
    uri.push_str(rest);
    uri
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn no_documented_endpoint_fails_with_a_server_error_on_hostile_input() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(state.clone())
        .merge(patient_administration_system::api::fhir::fhir_router(state));
    let mut failures = Vec::new();
    for (method, template) in ENDPOINTS {
        let base = fill(template);
        let person = Uuid::new_v4();
        let calls: Vec<(String, Option<String>)> = if *method == "GET" || *method == "DELETE" {
            QUERIES
                .iter()
                .map(|query| (format!("{base}{query}"), None))
                .collect()
        } else {
            bodies(person)
                .into_iter()
                .map(|body| (base.clone(), Some(body)))
                .collect()
        };
        for (uri, body) in calls {
            let mut request = Request::builder().method(*method).uri(&uri);
            if *method == "PUT" || *method == "PATCH" || *method == "DELETE" {
                request = request.header("if-match", "\"1\"");
            }
            let payload = match body {
                Some(text) => {
                    request = request.header("content-type", "application/json");
                    Body::from(text)
                }
                None => Body::empty(),
            };
            let response = app
                .clone()
                .oneshot(request.body(payload).unwrap())
                .await
                .unwrap();
            let status = response.status();
            if !acceptable(status) {
                let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                failures.push(format!(
                    "{method} {uri}: {status} {}",
                    String::from_utf8_lossy(&bytes)
                        .chars()
                        .take(200)
                        .collect::<String>()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
