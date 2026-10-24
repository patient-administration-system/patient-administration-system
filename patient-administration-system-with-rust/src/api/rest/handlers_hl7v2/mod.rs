//! HL7 v2 inbound handlers: the PID-bearing core below, then the other ADT variants, DFT, SIU and MFN
//! (see the route table).
//!
//! The first five endpoints are:
//! - `POST /api/hl7/v2/parse` — passive parse → JSON echo (no
//!   side-effects, helpful for client debugging).
//! - `POST /api/hl7/v2/patient` — ADT^A28 add-person. PID is parsed,
//!   then [`PersonClient::search`] looks for the MRN, and the
//!   handler either reuses the matched person id or creates a new one
//!   via [`PersonClient::create`].
//! - `POST /api/hl7/v2/admit` — ADT^A01. Same PID resolve, then look
//!   up the bed by PV1-3.3 code, then call [`AdtService::admit`].
//! - `POST /api/hl7/v2/transfer` — ADT^A02. Resolve PID to person id,
//!   find the open admission for that person, look up the destination
//!   bed by PV1-3.3, call [`AdtService::transfer`].
//! - `POST /api/hl7/v2/discharge` — ADT^A03. Resolve PID → person, find
//!   open admission, call [`AdtService::discharge`].
//!
//! Every handler returns a v2 ACK envelope (HTTP 200 + `text/plain`)
//! using the existing [`crate::hl7v2::ack::ack`] builder. AA/AE/AR
//! semantics:
//! - **AA** on success.
//! - **AE** on application-level errors (no PID, unknown bed, etc.).
//! - **AR** on parser failure or malformed wire payload.
//!
//! The other ADT variants, DFT^P03, SIU^S12 to S15 and MFN^M02/M05 follow the same
//! parse → resolve-person-upstream → call-PAS-service → ACK shape.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::IntoResponse,
};

use sea_orm::TransactionTrait;

use crate::api::rest::AppState;
use crate::api::rest::handlers::request_audit_ctx;
use crate::db::repositories::admission::AdmissionRepository;
use crate::db::repositories::audit::{AuditLogRepository, UserContext as RepoUserContext};
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::encounter::EncounterRepository;
use crate::db::repositories::outbox::OutboxRepository;
use crate::error::client_text;
use crate::hl7v2::SegmentRead;
use crate::hl7v2::ack::{AckCode, ack};
use crate::hl7v2::batch::{encode_batch_ack, parse_batch};
use crate::hl7v2::encoder::encode_message;
use crate::hl7v2::parser::parse_message;
use crate::hl7v2::pid_translation::{
    extract_bed_code_from_pv1, extract_mrn_from_mrg, extract_mrn_from_pid, parse_ft1,
    parse_mfe_loc, parse_mfe_stf, parse_sch, pid_to_person_create, pv1_class_or_outpatient,
};
use crate::models::appointment::{Appointment, AppointmentStatus, CancellationReason};
use crate::models::billing::{Account, Charge};
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::models::facility::BedStatus;
use crate::models::{Iso4217, Money};
use crate::upstream::{
    PersonClient, UserContext as UpstreamUserContext, WorkerClient, WorkerCreate,
};

mod adt;
mod adt_changes;
mod batches;
mod billing;
mod master;
mod scheduling;
pub use adt::*;
pub use adt_changes::*;
pub use batches::*;
pub use billing::*;
pub use master::*;
pub use scheduling::*;

const TEXT_PLAIN: [(axum::http::HeaderName, &str); 1] = [(CONTENT_TYPE, "text/plain")];
const PAS_APP: &str = "PAS";

/// Response shape every handler returns. The string is the v2 ACK
/// frame; the HTTP status code is 200 unless the wire-level parser
/// failed (in which case we still want to return the AR ACK so the
/// caller learns the diagnostics).
type Hl7Response = (
    StatusCode,
    [(axum::http::HeaderName, &'static str); 1],
    String,
);

fn ar(sender: &str, control_id: &str, message: &str) -> Hl7Response {
    (
        StatusCode::OK,
        TEXT_PLAIN,
        ack(PAS_APP, sender, control_id, AckCode::Reject, Some(message)),
    )
}

fn ae(sender: &str, control_id: &str, message: &str) -> Hl7Response {
    (
        StatusCode::OK,
        TEXT_PLAIN,
        ack(
            PAS_APP,
            sender,
            control_id,
            AckCode::AppError,
            Some(message),
        ),
    )
}

fn aa(sender: &str, control_id: &str, diag: Option<&str>) -> Hl7Response {
    (
        StatusCode::OK,
        TEXT_PLAIN,
        ack(PAS_APP, sender, control_id, AckCode::Accept, diag),
    )
}

/// Parse the message and pull the sending app + message control id from
/// MSH-3 / MSH-10. Used so every ACK reflects the inbound identity.
struct MshHeader {
    sender: String,
    control_id: String,
}

fn msh_header(msg: &crate::hl7v2::Message) -> MshHeader {
    let msh = msg.segment("MSH");
    MshHeader {
        sender: msh.map(|s| s.raw(3).to_string()).unwrap_or_default(),
        control_id: msh.map(|s| s.raw(10).to_string()).unwrap_or_default(),
    }
}

fn upstream_ctx(uc: &RepoUserContext) -> UpstreamUserContext {
    UpstreamUserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    }
}

fn need_person_client(state: &AppState) -> Option<&dyn PersonClient> {
    state.person_client.as_deref()
}

/// Look up an existing upstream person by MRN, or create one from the PID.
async fn dedup_or_create_person(
    client: &dyn PersonClient,
    pid: &crate::hl7v2::Segment,
    upstream_ctx: &UpstreamUserContext,
) -> Result<(uuid::Uuid, bool), crate::Error> {
    let mrn = extract_mrn_from_pid(pid)
        .ok_or_else(|| crate::Error::validation("PID-3.1 (MRN) is required"))?;

    // Best-effort dedup: search by MRN and look for an identifier-value
    // match in the result set. The upstream's search is full-text over
    // name + identifier values, so an MRN search usually returns
    // matches when the person already exists.
    let hits = client.search(&mrn, 20, upstream_ctx).await?;
    for candidate in &hits {
        if person_has_identifier_value(candidate, &mrn) {
            return Ok((candidate.id, false));
        }
    }

    // No match — create.
    let person = client
        .create(pid_to_person_create(pid), upstream_ctx)
        .await?;
    Ok((person.id, true))
}

fn person_has_identifier_value(p: &crate::upstream::Person, target: &str) -> bool {
    let Some(arr) = p.data.get("identifiers").and_then(|v| v.as_array()) else {
        return false;
    };
    arr.iter().any(|id| {
        id.get("value")
            .and_then(|v| v.as_str())
            .map(|s| s == target)
            .unwrap_or(false)
    })
}

// ─── /api/hl7/v2/parse ──────────────────────────────────────────────────

/// Passive parse: parse the wire, ACK AA with no side-effects. Useful
/// for client debugging.
#[utoipa::path(
    post,
    path = "/api/hl7/v2/parse",
    tag = "Hl7",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn parse(body: String) -> impl IntoResponse {
    match parse_message(&body) {
        Ok(msg) => {
            let h = msh_header(&msg);
            aa(&h.sender, &h.control_id, Some("parsed"))
        }
        Err(e) => ar("unknown", "unknown", &e.to_string()),
    }
}

// ─── /api/hl7/v2/patient (ADT^A28 add person) ──────────────────────────

// ─── /api/hl7/v2/admit (ADT^A01) ───────────────────────────────────────

// ─── /api/hl7/v2/transfer (ADT^A02) ────────────────────────────────────

// ─── /api/hl7/v2/discharge (ADT^A03) ───────────────────────────────────

// ─── Helpers shared across the ADT handlers ─────────────────────────────

/// Walk the parse → header → PID → person_id pipeline used by every
/// PID-driven handler. Returns either a populated context or a fully
/// built ACK ready to ship back.
async fn resolve_person_or_ack(
    state: &AppState,
    headers: &HeaderMap,
    body: &str,
) -> Result<ResolvedHeader, Hl7Response> {
    let msg = parse_message(body).map_err(|e| ar("unknown", "unknown", &e.to_string()))?;
    let h = msh_header(&msg);
    let pid = msg
        .segment("PID")
        .ok_or_else(|| ae(&h.sender, &h.control_id, "PID segment missing"))?
        .clone();
    let client = need_person_client(state).ok_or_else(|| {
        ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        )
    })?;
    let audit = request_audit_ctx(headers);
    let up = upstream_ctx(&audit);
    let (person_id, _created) = dedup_or_create_person(client, &pid, &up)
        .await
        .map_err(|e| ae(&h.sender, &h.control_id, &client_text(&e)))?;
    Ok(ResolvedHeader {
        message: msg,
        sender: h.sender,
        control_id: h.control_id,
        pid,
        person_id,
        audit,
        upstream: up,
    })
}

struct ResolvedHeader {
    message: crate::hl7v2::Message,
    sender: String,
    control_id: String,
    pid: crate::hl7v2::Segment,
    person_id: uuid::Uuid,
    audit: RepoUserContext,
    upstream: UpstreamUserContext,
}

// ─── /api/hl7/v2/register (ADT^A04 register outpatient) ────────────────

// ─── /api/hl7/v2/pre-admit (ADT^A05) ───────────────────────────────────

// ─── /api/hl7/v2/cancel-pre-admit (ADT^A38) ────────────────────────────

// ─── /api/hl7/v2/change-to-inpatient (ADT^A06) ─────────────────────────

// ─── /api/hl7/v2/change-to-outpatient (ADT^A07) ────────────────────────

// ─── /api/hl7/v2/update (ADT^A08 update patient info) ──────────────────

// ─── /api/hl7/v2/cancel-admit (ADT^A11) ────────────────────────────────

// ─── /api/hl7/v2/cancel-transfer (ADT^A12) ─────────────────────────────

// ─── /api/hl7/v2/cancel-discharge (ADT^A13) ────────────────────────────

// ─── /api/hl7/v2/leave-start (ADT^A21) ─────────────────────────────────

// ─── /api/hl7/v2/leave-end (ADT^A22) ───────────────────────────────────

// ─── /api/hl7/v2/delete-patient (ADT^A23) ──────────────────────────────

// ─── /api/hl7/v2/merge (ADT^A40) ───────────────────────────────────────

// ─── /api/hl7/v2/dft (DFT^P03 financial transaction) ────────────────────

// ─── /api/hl7/v2/schedule-book (SIU^S12) ───────────────────────────────

// ─── /api/hl7/v2/schedule-reschedule (SIU^S13) ─────────────────────────

// ─── /api/hl7/v2/schedule-modify (SIU^S14) ─────────────────────────────

// ─── /api/hl7/v2/schedule-cancel (SIU^S15) ─────────────────────────────

// ─── /api/hl7/v2/mfn-staff (MFN^M02) ───────────────────────────────────

// ─── /api/hl7/v2/mfn-location (MFN^M05) ────────────────────────────────

// ─── /api/hl7/v2/batch (FHS/BHS envelope dispatcher) ───────────────────
