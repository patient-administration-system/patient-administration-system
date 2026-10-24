//! IHE ITI Scheduling (FHIR R4): `$find`, `$hold` and `$book` on `Appointment` (NEW-10).
//!
//! The profile is FHIR R4 and the PAS endpoint is R5, so these live under `/fhir/r4` with their own
//! CapabilityStatement (`/fhir/r4/metadata`) and are not in the R5 statement. Read from
//! <https://profiles.ihe.net/ITI/Scheduling/> (ITI-115, ITI-116, ITI-117 and the three OperationDefinitions)
//! on 2026-10-23.
//!
//! Supported: `$find` with `start`, `end`, `practitioner`, `location-reference` (a room or bed) and
//! `patient-reference` (required, one patient), `_count`; `$hold` and `$book` with `appointment-reference`
//! (the id of a proposal from `$find`), and `comment`. Any other parameter is refused by name rather than
//! ignored. **Not built:** modify and cancel through `$book` (`appointment-resource`), a patient who does
//! not exist yet (`patient-resource`), group appointments, specialty, visit type, organization, reason,
//! referral, timing and insurance filters, ITI-118, and paging.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

use crate::api::rest::AppState;
use crate::api::rest::handlers::request_audit_ctx;
use crate::config::env;
use crate::scheduling::ihe::{FindQuery, Proposal};
use crate::{Error, Result};

const CONTENT_TYPE: &str = "application/fhir+json; fhirVersion=4.0";

fn respond(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, CONTENT_TYPE)],
        body.to_string(),
    )
        .into_response()
}

fn outcome(status: StatusCode, severity: &str, code: &str, diagnostics: &str) -> Response {
    respond(
        status,
        json!({
            "resourceType": "OperationOutcome",
            "issue": [{ "severity": severity, "code": code, "diagnostics": diagnostics }],
        }),
    )
}

/// Map a service error onto an OperationOutcome. Database and internal errors are not shown.
fn failure(e: &Error) -> Response {
    match e {
        Error::NotFound(_) => outcome(
            StatusCode::NOT_FOUND,
            "fatal",
            "not-found",
            "that appointment is no longer available",
        ),
        Error::Validation(_) => outcome(
            StatusCode::BAD_REQUEST,
            "error",
            "invalid",
            &e.client_message(),
        ),
        Error::Conflict(_) | Error::InvalidStateTransition(_) => outcome(
            StatusCode::CONFLICT,
            "error",
            "conflict",
            &e.client_message(),
        ),
        _ => {
            tracing::error!(error = %e, "scheduling operation failed");
            outcome(
                StatusCode::INTERNAL_SERVER_ERROR,
                "fatal",
                "exception",
                "the request could not be completed",
            )
        }
    }
}

/// The parameters of a `Parameters` resource, by name.
struct Params(Vec<(String, Value)>);

impl Params {
    fn parse(body: &Value) -> Result<Self> {
        if body["resourceType"] != "Parameters" {
            return Err(Error::validation("the body must be a Parameters resource"));
        }
        let list = body["parameter"].as_array().cloned().unwrap_or_default();
        Ok(Self(
            list.into_iter()
                .filter_map(|p| Some((p["name"].as_str()?.to_string(), p)))
                .collect(),
        ))
    }

    /// Refuse a parameter the operation does not support, naming it.
    fn only(&self, supported: &[&str]) -> Result<()> {
        let mut bad: Vec<&str> = self
            .0
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| !supported.contains(n))
            .collect();
        bad.sort_unstable();
        bad.dedup();
        if bad.is_empty() {
            Ok(())
        } else {
            Err(Error::validation(format!(
                "not supported: {}; supported: {}",
                bad.join(", "),
                supported.join(", ")
            )))
        }
    }

    fn all(&self, name: &str) -> Vec<&Value> {
        self.0
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v)
            .collect()
    }

    fn one(&self, name: &str) -> Result<Option<&Value>> {
        let all = self.all(name);
        if all.len() > 1 {
            return Err(Error::validation(format!("{name} may be given once")));
        }
        Ok(all.first().copied())
    }

    fn time(&self, name: &str) -> Result<Option<DateTime<Utc>>> {
        let Some(v) = self.one(name)? else {
            return Ok(None);
        };
        v["valueDateTime"]
            .as_str()
            .or_else(|| v["valueInstant"].as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| Some(t.with_timezone(&Utc)))
            .ok_or_else(|| Error::validation(format!("{name} must be a dateTime with a time zone")))
    }

    /// A reference to `kind` (`Patient/<uuid>` or a bare uuid), at most once.
    fn reference(&self, name: &str, kind: &str) -> Result<Option<Uuid>> {
        let Some(v) = self.one(name)? else {
            return Ok(None);
        };
        v["valueReference"]["reference"]
            .as_str()
            .map(|r| r.strip_prefix(&format!("{kind}/")).unwrap_or(r))
            .and_then(|r| Uuid::parse_str(r).ok())
            .map(Some)
            .ok_or_else(|| {
                Error::validation(format!("{name} must be a reference like {kind}/<id>"))
            })
    }
}

/// A proposal, hold or booking as an R4 `Appointment`. `proposed` while only offered, `pending` while held,
/// `booked` once booked.
pub fn to_appointment(p: &Proposal) -> Value {
    let (status, id, accepted) = match (p.status.as_str(), p.appointment_id) {
        ("booked", Some(a)) => ("booked", a, true),
        ("held", _) => ("pending", p.id, false),
        _ => ("proposed", p.id, false),
    };
    let owner = match p.owner_kind.as_str() {
        "practitioner" => Some(format!("Practitioner/{}", p.owner_id)),
        "room" | "bed" => Some(format!("Location/{}", p.owner_id)),
        _ => None,
    };
    let part = if accepted { "accepted" } else { "needs-action" };
    let mut participants = vec![json!({
        "actor": { "reference": format!("Patient/{}", p.person_id) }, "status": part,
    })];
    if let Some(r) = owner {
        participants.push(json!({ "actor": { "reference": r }, "status": part }));
    }
    json!({
        "resourceType": "Appointment",
        "id": id,
        "status": status,
        "start": p.start,
        "end": p.end,
        "slot": [{ "reference": format!("Slot/{}", p.slot_id) }],
        "requestedPeriod": [{
            "start": p.requested.map_or(p.start, |r| r.0),
            "end": p.requested.map_or(p.end, |r| r.1),
        }],
        "participant": participants,
    })
}

/// The address this R4 endpoint answers at: `PAS_FHIR_BASE_URL` (no trailing slash) when set, otherwise built
/// from the request's `Host` (and `X-Forwarded-Proto`). A Bundle entry's `fullUrl` and the relative references
/// inside it are resolved against it.
fn base_url(headers: &HeaderMap) -> String {
    if let Some(b) = crate::config::env::text("PAS_FHIR_BASE_URL") {
        return b.trim_end_matches('/').to_string();
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|h| !h.trim().is_empty())
        .unwrap_or("localhost");
    let scheme = match headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
    {
        Some("https") => "https",
        _ => "http",
    };
    format!("{scheme}://{host}/fhir/r4")
}

/// A `searchset` Bundle of Appointments. Each entry has the `fullUrl` and `search.mode` a searchset needs,
/// and the Bundle a `self` link naming the operation that made it.
fn bundle(appointments: &[Proposal], base: &str, operation: &str) -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "searchset",
        "total": appointments.len(),
        "link": [{ "relation": "self", "url": format!("{base}/Appointment/{operation}") }],
        "entry": appointments.iter().map(|p| {
            let resource = to_appointment(p);
            json!({
                "fullUrl": format!("{base}/Appointment/{}", resource["id"].as_str().unwrap_or_default()),
                "resource": resource,
                "search": { "mode": "match" },
            })
        }).collect::<Vec<_>>(),
    })
}

/// `POST /fhir/r4/Appointment/$find` (ITI-115): proposed appointments, earliest first.
#[utoipa::path(
    post,
    path = "/fhir/r4/Appointment/$find",
    tag = "FHIR",
    summary = "IHE ITI-115 Find Potential Appointments (FHIR R4)",
    responses(
        (status = 200, description = "FHIR R4 resource or Bundle (`application/fhir+json; fhirVersion=4.0`)"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server error; an OperationOutcome"),
    )
)]
pub async fn find(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let run = async {
        let p = Params::parse(&body)?;
        p.only(&[
            "start",
            "end",
            "practitioner",
            "location-reference",
            "patient-reference",
        ])?;
        let now = state.clock.now();
        let start = p.time("start")?.unwrap_or(now);
        let end = p.time("end")?.unwrap_or(start + Duration::days(14));
        let patient = p
            .reference("patient-reference", "Patient")?
            .ok_or_else(|| Error::validation("patient-reference is required: Patient/<id>"))?;
        let count = match q.get("_count") {
            Some(c) => c
                .parse::<i64>()
                .ok()
                .filter(|c| *c >= 1)
                .ok_or_else(|| Error::validation("_count must be a positive number"))?,
            None => 10,
        };
        let query = FindQuery {
            patient,
            start,
            end,
            practitioner: p.reference("practitioner", "Practitioner")?,
            location: p.reference("location-reference", "Location")?,
            count,
        };
        state
            .scheduling
            .find_proposals(&query, now, &request_audit_ctx(&headers))
            .await
    };
    match run.await {
        Ok(found) => respond(StatusCode::OK, bundle(&found, &base_url(&headers), "$find")),
        Err(e) => failure(&e),
    }
}

fn proposal_id(p: &Params) -> Result<Uuid> {
    p.reference("appointment-reference", "Appointment")?
        .ok_or_else(|| Error::validation("appointment-reference is required: Appointment/<id>"))
}

/// `POST /fhir/r4/Appointment/$hold` (ITI-116): the appointment comes back `pending` for `PAS_HOLD_MINUTES`.
#[utoipa::path(
    post,
    path = "/fhir/r4/Appointment/$hold",
    tag = "FHIR",
    summary = "IHE ITI-116 Hold Appointment (FHIR R4)",
    responses(
        (status = 200, description = "FHIR R4 resource or Bundle (`application/fhir+json; fhirVersion=4.0`)"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server error; an OperationOutcome"),
    )
)]
pub async fn hold(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let run = async {
        let p = Params::parse(&body)?;
        p.only(&["appointment-reference"])?;
        let id = proposal_id(&p)?;
        let ttl = Duration::minutes(env::int("PAS_HOLD_MINUTES"));
        state
            .scheduling
            .hold_proposal(id, state.clock.now(), ttl, &request_audit_ctx(&headers))
            .await
    };
    match run.await {
        Ok(held) => respond(
            StatusCode::OK,
            bundle(&[held], &base_url(&headers), "$hold"),
        ),
        Err(e) => failure(&e),
    }
}

/// `POST /fhir/r4/Appointment/$book` (ITI-117): book a proposal; the appointment comes back `booked`.
#[utoipa::path(
    post,
    path = "/fhir/r4/Appointment/$book",
    tag = "FHIR",
    summary = "IHE ITI-117 Book Appointment (FHIR R4)",
    responses(
        (status = 200, description = "FHIR R4 resource or Bundle (`application/fhir+json; fhirVersion=4.0`)"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server error; an OperationOutcome"),
    )
)]
pub async fn book(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let run = async {
        let p = Params::parse(&body)?;
        p.only(&["appointment-reference", "comment"])?;
        let id = proposal_id(&p)?;
        state
            .scheduling
            .book_proposal(id, state.clock.now(), &request_audit_ctx(&headers))
            .await
    };
    match run.await {
        Ok((proposal, _)) => respond(
            StatusCode::OK,
            bundle(&[proposal], &base_url(&headers), "$book"),
        ),
        Err(e) => failure(&e),
    }
}

/// `GET /fhir/r4/metadata`: an R4 CapabilityStatement that lists only the three operations.
#[utoipa::path(
    get,
    path = "/fhir/r4/metadata",
    tag = "FHIR",
    summary = "CapabilityStatement for the IHE Scheduling operations (FHIR R4)",
    responses(
        (status = 200, description = "FHIR R4 resource or Bundle (`application/fhir+json; fhirVersion=4.0`)"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server error; an OperationOutcome"),
    )
)]
pub async fn metadata(headers: HeaderMap) -> Response {
    respond(
        StatusCode::OK,
        json!({
            "resourceType": "CapabilityStatement",
            "status": "active",
            "date": "2026-10-23",
            "kind": "instance",
            "software": { "name": "patient-administration-system", "version": env!("CARGO_PKG_VERSION") },
            "implementation": {
                "description": "Patient Administration System: IHE ITI Scheduling (FHIR R4)",
                "url": base_url(&headers),
            },
            "fhirVersion": "4.0.1",
            "format": ["json"],
            "rest": [{
                "mode": "server",
                "resource": [{
                    "type": "Appointment",
                    "operation": [
                        { "name": "find", "definition": "https://profiles.ihe.net/ITI/Scheduling/OperationDefinition/appointment-find" },
                        { "name": "hold", "definition": "https://profiles.ihe.net/ITI/Scheduling/OperationDefinition/appointment-hold" },
                        { "name": "book", "definition": "https://profiles.ihe.net/ITI/Scheduling/OperationDefinition/appointment-book" },
                    ],
                }],
            }],
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(status: &str, appointment: Option<Uuid>) -> Proposal {
        Proposal {
            id: Uuid::new_v4(),
            slot_id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            start: Utc::now(),
            end: Utc::now() + Duration::minutes(15),
            status: status.into(),
            expires_at: Utc::now(),
            appointment_id: appointment,
            owner_kind: "practitioner".into(),
            owner_id: Uuid::new_v4(),
            requested: None,
        }
    }

    #[test]
    fn status_follows_the_proposal_and_a_booking_carries_the_real_appointment_id() {
        let booked = Uuid::new_v4();
        let a = to_appointment(&proposal("proposed", None));
        assert_eq!(a["status"], "proposed");
        assert_eq!(to_appointment(&proposal("held", None))["status"], "pending");
        let b = to_appointment(&proposal("booked", Some(booked)));
        assert_eq!(b["status"], "booked");
        assert_eq!(b["id"], booked.to_string());
        assert_eq!(b["participant"][0]["status"], "accepted");
        assert!(
            b["participant"][1]["actor"]["reference"]
                .as_str()
                .unwrap()
                .starts_with("Practitioner/")
        );
    }

    #[test]
    fn parameters_refuse_what_is_not_supported_by_name() {
        let body = json!({"resourceType": "Parameters", "parameter": [
            {"name": "start", "valueDateTime": "2026-11-01T09:00:00Z"},
            {"name": "specialty", "valueString": "cardiology"},
            {"name": "referral-identifier", "valueString": "x"},
        ]});
        let p = Params::parse(&body).unwrap();
        let e = p.only(&["start", "end"]).unwrap_err().to_string();
        assert!(e.contains("referral-identifier, specialty"), "{e}");
        assert!(Params::parse(&json!({"resourceType": "Patient"})).is_err());
    }

    #[test]
    fn references_and_times_are_checked() {
        let body = json!({"resourceType": "Parameters", "parameter": [
            {"name": "patient-reference", "valueReference": {"reference": "Patient/not-a-uuid"}},
            {"name": "start", "valueDateTime": "tomorrow"},
            {"name": "end", "valueDateTime": "2026-11-01T09:00:00+00:00"},
        ]});
        let p = Params::parse(&body).unwrap();
        assert!(p.reference("patient-reference", "Patient").is_err());
        assert!(p.time("start").is_err());
        assert!(p.time("end").unwrap().is_some());
        assert!(p.time("missing").unwrap().is_none());
    }
}
