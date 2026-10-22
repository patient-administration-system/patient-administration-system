//! FHIR R5 HTTP handlers.
//!
//! `/fhir/Patient/*` and `/fhir/Practitioner/*` proxy `person-service` / `worker-service` and forward their
//! shape verbatim. Encounter, Appointment, Coverage, Schedule, Slot and Location are read from the PAS-owned
//! tables and wrapped as FHIR; Schedule and Slot can also be written. `POST /fhir/Encounter` and
//! `POST /fhir/Appointment` are intentionally not exposed (`503` with a `not-supported` OperationOutcome that
//! names the REST endpoint to use). `POST /fhir` accepts a write Bundle.
//!
//! ## Wire shape
//!
//! Patient and Practitioner forward the upstream's Person / Worker shape unchanged; the PAS does not
//! re-translate it. Callers that need FHIR R5 Person semantics should use `person-service` directly; this
//! surface is a compatibility shim.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::upstream::{PersonClient, PersonCreate, WorkerClient, WorkerCreate};

use super::operation_outcome::{self, OperationOutcome};

type FhirResult = Result<Json<Value>, (StatusCode, Json<OperationOutcome>)>;

/// Map any [`Error`] to a status + OperationOutcome.
/// A resource as JSON. One that cannot be serialized is a logged `500`, never a silent `null`.
fn json_of<T: serde::Serialize>(value: T) -> FhirResult {
    serde_json::to_value(value).map(Json).map_err(|e| {
        error_to_response(Error::internal(format!(
            "response could not be serialized: {e}"
        )))
    })
}

/// A resource wrapped as FHIR (`wrap_fhir`); one that cannot be serialized is a logged `500`.
fn fhir_of<T: serde::Serialize>(kind: &str, value: T) -> FhirResult {
    match serde_json::to_value(value) {
        Ok(v) => Ok(Json(wrap_fhir(kind, v))),
        Err(e) => Err(error_to_response(Error::internal(format!(
            "response could not be serialized: {e}"
        )))),
    }
}

fn error_to_response(err: Error) -> (StatusCode, Json<OperationOutcome>) {
    let (status, code) = match &err {
        Error::NotFound(_) => (StatusCode::NOT_FOUND, "not-found"),
        Error::Validation(_) | Error::Fhir(_) => (StatusCode::BAD_REQUEST, "invalid"),
        Error::InvalidStateTransition(_) | Error::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
        Error::Upstream(_) => (StatusCode::SERVICE_UNAVAILABLE, "transient"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "exception"),
    };
    (
        status,
        Json(operation_outcome::error(code, err.to_string())),
    )
}

fn need_person_client(state: &AppState) -> Result<&dyn PersonClient, Error> {
    state
        .person_client
        .as_deref()
        .ok_or_else(|| Error::upstream("PERSON_SERVICE_URL unset; FHIR Patient proxy disabled"))
}

fn need_worker_client(state: &AppState) -> Result<&dyn WorkerClient, Error> {
    state.worker_client.as_deref().ok_or_else(|| {
        Error::upstream("WORKER_SERVICE_URL unset; FHIR Practitioner proxy disabled")
    })
}

fn body_to_fields(body: Value) -> Result<serde_json::Map<String, Value>, Error> {
    match body {
        Value::Object(m) => Ok(m),
        other => Err(Error::validation(format!(
            "FHIR body must be a JSON object, got {other:?}"
        ))),
    }
}

// ─── /fhir/Patient/* (proxy person-service) ────────────────────────

// ─── /fhir/Practitioner/* (proxy worker-service) ───────────────────

// ─── PAS-owned FHIR reads ───────────────────────────────────────────────
//
// These return JSON wrapped in a minimal FHIR-shaped envelope. They
// don't translate to typed FhirEncounter / FhirAppointment / FhirCoverage
// shapes. The wire shape is the underlying PAS row JSON with a
// `resourceType` field added so basic FHIR client type-routing works.
// A real translator (and the R4 form Wales uses) is NEW-1 in `tasks.md`.

use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::coverage::CoverageRepository;
use crate::db::repositories::encounter::EncounterRepository;
use crate::db::repositories::schedule::ScheduleRepository;
use crate::db::repositories::slot::SlotRepository;

// ─── Write surface ──────────────────────────────────────────────────────
//
// FHIR Schedule + Slot create/update/delete persist directly against the
// PAS-owned `schedules` / `slots` tables. Bundle POST validates entries
// for Decision #4 (no Patient or Practitioner entries — those must go
// through person-service / worker-service directly) and dispatches each
// remaining entry by `resourceType`. Encounter + Appointment create are
// left stubbed because both need person-id resolution / state-machine
// orchestration (ADT for Encounter, scheduling service for Appointment)
// — callers should use the typed REST endpoints (`/api/admissions`,
// `/api/appointments`) for those.

use crate::models::schedule::{Schedule, ScheduleOwner, Slot, SlotStatus};

mod bundle;
mod people;
mod reads;
mod schedules;
pub use bundle::*;
pub use people::*;
pub use reads::*;
pub use schedules::*;

// ── Schedule writes ──────────────────────────────────────────────────────

// ── Slot writes ──────────────────────────────────────────────────────────

// ── Bundle POST with Decision #4 enforcement ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_schedule_owner_accepts_fhir_actor_practitioner() {
        let id = Uuid::new_v4();
        let body = json!({
            "actor": [{ "reference": format!("Practitioner/{id}") }]
        });
        match parse_schedule_owner(&body).unwrap() {
            ScheduleOwner::Practitioner(got) => assert_eq!(got, id),
            other => panic!("expected Practitioner, got {other:?}"),
        }
    }

    #[test]
    fn parse_schedule_owner_accepts_fhir_actor_location_as_bed() {
        let id = Uuid::new_v4();
        let body = json!({
            "actor": [{ "reference": format!("Location/{id}") }]
        });
        match parse_schedule_owner(&body).unwrap() {
            ScheduleOwner::Bed(got) => assert_eq!(got, id),
            other => panic!("expected Bed (mapped from Location), got {other:?}"),
        }
    }

    #[test]
    fn parse_schedule_owner_accepts_pas_native_shape() {
        let id = Uuid::new_v4();
        let body = json!({ "owner": { "kind": "Room", "id": id.to_string() } });
        match parse_schedule_owner(&body).unwrap() {
            ScheduleOwner::Room(got) => assert_eq!(got, id),
            other => panic!("expected Room, got {other:?}"),
        }
    }

    #[test]
    fn parse_schedule_owner_rejects_missing() {
        let err = parse_schedule_owner(&json!({})).unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn parse_schedule_owner_rejects_unknown_kind() {
        let id = Uuid::new_v4();
        let body = json!({ "actor": [{ "reference": format!("Organization/{id}") }] });
        let err = parse_schedule_owner(&body).unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn parse_service_type_prefers_pas_native_then_fhir_then_default() {
        assert_eq!(
            parse_service_type(&json!({ "service_type": "cardiology" })),
            "cardiology"
        );
        assert_eq!(
            parse_service_type(&json!({
                "serviceType": [{ "concept": { "coding": [{ "code": "PHYS" }] } }]
            })),
            "PHYS"
        );
        assert_eq!(parse_service_type(&json!({})), "general");
    }

    #[test]
    fn parse_slot_status_maps_fhir_strings() {
        assert_eq!(
            parse_slot_status(&json!({ "status": "free" })).unwrap(),
            Some(SlotStatus::Free)
        );
        assert_eq!(
            parse_slot_status(&json!({ "status": "busy" })).unwrap(),
            Some(SlotStatus::Busy)
        );
        assert_eq!(
            parse_slot_status(&json!({ "status": "busy-unavailable" })).unwrap(),
            Some(SlotStatus::BlockedOut)
        );
        assert!(parse_slot_status(&json!({})).unwrap().is_none());
        assert!(parse_slot_status(&json!({ "status": "garbage" })).is_err());
    }

    #[test]
    fn parse_schedule_ref_accepts_both_shapes() {
        let id = Uuid::new_v4();
        assert_eq!(
            parse_schedule_ref(&json!({ "schedule_id": id.to_string() })).unwrap(),
            id
        );
        assert_eq!(
            parse_schedule_ref(&json!({
                "schedule": { "reference": format!("Schedule/{id}") }
            }))
            .unwrap(),
            id
        );
    }

    #[test]
    fn parse_schedule_ref_rejects_bad_prefix() {
        let id = Uuid::new_v4();
        let err = parse_schedule_ref(&json!({
            "schedule": { "reference": format!("Provenance/{id}") }
        }))
        .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn parse_dt_round_trips_rfc3339() {
        let v = json!({ "start": "2026-06-01T09:00:00Z" });
        let got = parse_dt(&v, "start").unwrap();
        assert_eq!(got.format("%Y%m%d%H%M%S").to_string(), "20260601090000");
    }

    #[test]
    fn parse_dt_rejects_garbage() {
        let v = json!({ "start": "not a date" });
        assert!(parse_dt(&v, "start").is_err());
    }
}
