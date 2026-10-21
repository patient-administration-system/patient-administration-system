//! HL7 v2 inbound handlers — Phase 3c minimum viable PID-bearing surface.
//!
//! Five endpoints land here:
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
//! The remaining 14 ADT variants + DFT^P03 + SIU^S12/S13/S14/S15 +
//! MFN^M02/M05 land in Phase 3c2 — they all follow the same
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

#[utoipa::path(
    post,
    path = "/api/hl7/v2/patient",
    tag = "Hl7",
    summary = "Add person",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn add_person(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let pid = match msg.segment("PID") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PID segment missing"),
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        );
    };
    let ctx = upstream_ctx(&request_audit_ctx(&headers));
    match dedup_or_create_person(client, pid, &ctx).await {
        Ok((id, created)) => {
            let diag = if created {
                format!("created person {id}")
            } else {
                format!("matched existing person {id}")
            };
            aa(&h.sender, &h.control_id, Some(&diag))
        }
        Err(e) => ae(&h.sender, &h.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/admit (ADT^A01) ───────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/admit",
    tag = "Hl7",
    summary = "Admit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let pid = match msg.segment("PID") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PID segment missing"),
    };
    let pv1 = match msg.segment("PV1") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PV1 segment missing"),
    };
    let bed_code = match extract_bed_code_from_pv1(pv1) {
        Some(c) => c,
        None => return ae(&h.sender, &h.control_id, "PV1-3.3 (bed code) is required"),
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        );
    };
    let audit = request_audit_ctx(&headers);
    let ctx = upstream_ctx(&audit);
    let (person_id, _) = match dedup_or_create_person(client, pid, &ctx).await {
        Ok(r) => r,
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &h.sender,
                &h.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    match state.adt.admit(person_id, bed.id, &audit).await {
        Ok(result) => aa(
            &h.sender,
            &h.control_id,
            Some(&format!(
                "admitted person={person_id} bed={} admission={}",
                bed.id, result.admission.id
            )),
        ),
        Err(e) => ae(&h.sender, &h.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/transfer (ADT^A02) ────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/transfer",
    tag = "Hl7",
    summary = "Transfer",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let pid = match msg.segment("PID") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PID segment missing"),
    };
    let pv1 = match msg.segment("PV1") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PV1 segment missing"),
    };
    let bed_code = match extract_bed_code_from_pv1(pv1) {
        Some(c) => c,
        None => {
            return ae(
                &h.sender,
                &h.control_id,
                "PV1-3.3 (destination bed) is required",
            );
        }
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        );
    };
    let audit = request_audit_ctx(&headers);
    let ctx = upstream_ctx(&audit);
    let (person_id, _) = match dedup_or_create_person(client, pid, &ctx).await {
        Ok(r) => r,
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &h.sender,
                &h.control_id,
                &format!("no open admission for person {person_id}"),
            );
        }
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &h.sender,
                &h.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    match state.adt.transfer(admission.id, bed.id, &audit).await {
        Ok(_) => aa(
            &h.sender,
            &h.control_id,
            Some(&format!(
                "transferred person={person_id} admission={} to bed={}",
                admission.id, bed.id
            )),
        ),
        Err(e) => ae(&h.sender, &h.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/discharge (ADT^A03) ───────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/discharge",
    tag = "Hl7",
    summary = "Discharge",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn discharge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let pid = match msg.segment("PID") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PID segment missing"),
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        );
    };
    let audit = request_audit_ctx(&headers);
    let ctx = upstream_ctx(&audit);
    let (person_id, _) = match dedup_or_create_person(client, pid, &ctx).await {
        Ok(r) => r,
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &h.sender,
                &h.control_id,
                &format!("no open admission for person {person_id}"),
            );
        }
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    match state.adt.discharge(admission.id, &audit).await {
        Ok(_) => aa(
            &h.sender,
            &h.control_id,
            Some(&format!(
                "discharged person={person_id} admission={}",
                admission.id
            )),
        ),
        Err(e) => ae(&h.sender, &h.control_id, &client_text(&e)),
    }
}

// ─── Helpers shared across the Phase 3c2 ADT handlers ──────────────────

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

#[utoipa::path(
    post,
    path = "/api/hl7/v2/register",
    tag = "Hl7",
    summary = "Register",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    // PV1-2 chooses Emergency vs Outpatient. Missing PV1 → Outpatient.
    let class_str = match r.message.segment("PV1") {
        Some(pv1) => pv1_class_or_outpatient(pv1),
        None => "Outpatient",
    };
    let class = if class_str == "Emergency" {
        EncounterClass::Emergency
    } else {
        EncounterClass::Outpatient
    };
    // Build an Encounter directly in `Arrived` status (no bed allocated).
    let mut enc = Encounter::new(r.person_id, class);
    enc.status = EncounterStatus::Arrived;
    let enc_clone = enc.clone();
    let audit_clone = r.audit.clone();
    let res = state
        .db
        .transaction::<_, Encounter, crate::Error>(|txn| {
            Box::pin(async move {
                let e = EncounterRepository::create(txn, &enc_clone).await?;
                AuditLogRepository::log(
                    txn,
                    "encounter",
                    e.id,
                    "register_via_hl7v2_a04",
                    None,
                    Some(serde_json::to_value(&e).unwrap_or_default()),
                    &audit_clone,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "EncounterRegistered",
                    &serde_json::json!({
                        "encounter_id": e.id,
                        "person_id": e.person_id,
                        "class": format!("{:?}", e.class),
                        "source": "hl7v2_a04",
                    }),
                )
                .await?;
                Ok(e)
            })
        })
        .await;
    match res {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "registered encounter={} person={} class={}",
                e.id, r.person_id, class_str
            )),
        ),
        Err(e) => ae(
            &r.sender,
            &r.control_id,
            &crate::Error::from(e).client_message(),
        ),
    }
}

// ─── /api/hl7/v2/pre-admit (ADT^A05) ───────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/pre-admit",
    tag = "Hl7",
    summary = "Pre admit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let bed_code = match r.message.segment("PV1").and_then(extract_bed_code_from_pv1) {
        Some(c) => c,
        None => return ae(&r.sender, &r.control_id, "PV1-3.3 (bed) is required"),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .pre_admit(r.person_id, bed.id, Some("hl7v2_a05"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "pre-admitted person={} encounter={} bed={}",
                r.person_id, e.id, bed.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/cancel-pre-admit (ADT^A38) ────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-pre-admit",
    tag = "Hl7",
    summary = "Cancel pre admit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let bed_code = match r.message.segment("PV1").and_then(extract_bed_code_from_pv1) {
        Some(c) => c,
        None => return ae(&r.sender, &r.control_id, "PV1-3.3 (bed) is required"),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .cancel_pre_admit(r.person_id, bed.id, Some("hl7v2_a38"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "cancelled pre-admit person={} encounter={} bed={}",
                r.person_id, e.id, bed.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/change-to-inpatient (ADT^A06) ─────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/change-to-inpatient",
    tag = "Hl7",
    summary = "Change to inpatient",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn change_to_inpatient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let bed_code = match r.message.segment("PV1").and_then(extract_bed_code_from_pv1) {
        Some(c) => c,
        None => return ae(&r.sender, &r.control_id, "PV1-3.3 (bed) is required"),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .change_to_inpatient(r.person_id, bed.id, Some("hl7v2_a06"), &r.audit)
        .await
    {
        Ok(result) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "promoted person={} encounter={} bed={}",
                r.person_id, result.encounter.id, bed.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/change-to-outpatient (ADT^A07) ────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/change-to-outpatient",
    tag = "Hl7",
    summary = "Change to outpatient",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn change_to_outpatient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .change_to_outpatient(admission.id, Some("hl7v2_a07"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "demoted person={} encounter={}",
                r.person_id, e.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/update (ADT^A08 update patient info) ──────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/update",
    tag = "Hl7",
    summary = "Update",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    // A08 overwrites the upstream person from the new PID. Use
    // `PersonClient::update` against the resolved person id.
    let Some(client) = need_person_client(&state) else {
        return ae(
            &r.sender,
            &r.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot push update",
        );
    };
    let create = pid_to_person_create(&r.pid);
    match client.update(r.person_id, create, &r.upstream).await {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("updated person={}", r.person_id)),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/cancel-admit (ADT^A11) ────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-admit",
    tag = "Hl7",
    summary = "Cancel admit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .cancel_admission(admission.id, Some("hl7v2_a11"), &r.audit)
        .await
    {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "cancelled admission={} person={}",
                admission.id, r.person_id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/cancel-transfer (ADT^A12) ─────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-transfer",
    tag = "Hl7",
    summary = "Cancel transfer",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .cancel_transfer(admission.id, Some("hl7v2_a12"), &r.audit)
        .await
    {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("reverted transfer on admission={}", admission.id)),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/cancel-discharge (ADT^A13) ────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-discharge",
    tag = "Hl7",
    summary = "Cancel discharge",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_discharge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let (admission, _discharge) =
        match AdmissionRepository::find_most_recently_discharged_for_patient(&state.db, r.person_id)
            .await
        {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                return ae(
                    &r.sender,
                    &r.control_id,
                    &format!("no discharged admission found for person {}", r.person_id),
                );
            }
            Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
        };
    let admission_id = admission.id;
    match state.adt.cancel_discharge(admission_id, &r.audit).await {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "reinstated discharge on admission={admission_id} person={}",
                r.person_id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/leave-start (ADT^A21) ─────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/leave-start",
    tag = "Hl7",
    summary = "Leave start",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn leave_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .start_leave(admission.id, Some("hl7v2_a21"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "leave-start admission={} encounter={}",
                admission.id, e.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/leave-end (ADT^A22) ───────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/leave-end",
    tag = "Hl7",
    summary = "Leave end",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn leave_end(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .end_leave(admission.id, Some("hl7v2_a22"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "leave-end admission={} encounter={}",
                admission.id, e.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/delete-patient (ADT^A23) ──────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/delete-patient",
    tag = "Hl7",
    summary = "Delete patient",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn delete_patient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &r.sender,
            &r.control_id,
            "PERSON_SERVICE_URL unset; cannot delete upstream person",
        );
    };
    match client.delete(r.person_id, &r.upstream).await {
        Ok(()) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("deleted person={}", r.person_id)),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/merge (ADT^A40) ───────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/merge",
    tag = "Hl7",
    summary = "Merge",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn merge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    // A40 carries the SURVIVOR in PID and the SOURCE MRN in MRG-1. The
    // upstream merge endpoint takes two ids — we resolve PID via
    // dedup-or-create and MRG via a search on the source MRN.
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let mrg = match r.message.segment("MRG") {
        Some(s) => s,
        None => return ae(&r.sender, &r.control_id, "MRG segment missing"),
    };
    let source_mrn = match extract_mrn_from_mrg(mrg) {
        Some(s) => s,
        None => return ae(&r.sender, &r.control_id, "MRG-1.1 (source MRN) is required"),
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &r.sender,
            &r.control_id,
            "PERSON_SERVICE_URL unset; cannot resolve MRG source",
        );
    };
    // Look up the source by MRN. We do NOT dedup-or-create the source —
    // it must already exist or the merge is rejected (no person to merge
    // from).
    let hits = match client.search(&source_mrn, 20, &r.upstream).await {
        Ok(h) => h,
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    let source_id = match hits
        .iter()
        .find(|p| {
            p.data
                .get("identifiers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|id| {
                        id.get("value")
                            .and_then(|v| v.as_str())
                            .map(|s| s == source_mrn)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        })
        .map(|p| p.id)
    {
        Some(id) => id,
        None => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("source MRN {source_mrn:?} not found upstream"),
            );
        }
    };
    if source_id == r.person_id {
        return ae(
            &r.sender,
            &r.control_id,
            "MRG-1 resolved to the same person as PID (self-merge)",
        );
    }
    match client.merge(source_id, r.person_id, &r.upstream).await {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "merged source={source_id} into target={}",
                r.person_id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/dft (DFT^P03 financial transaction) ────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/dft",
    tag = "Hl7",
    summary = "Dft",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn dft(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let ft1_segs: Vec<_> = r.message.segments_named("FT1").collect();
    if ft1_segs.is_empty() {
        return ae(&r.sender, &r.control_id, "no FT1 segments");
    }
    let mut parsed: Vec<crate::hl7v2::pid_translation::Ft1> = Vec::with_capacity(ft1_segs.len());
    let mut shared_currency: Option<String> = None;
    for seg in &ft1_segs {
        let ft1 = match parse_ft1(seg) {
            Ok(f) => f,
            Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
        };
        if !matches!(ft1.transaction_type.as_str(), "CG" | "") {
            return ae(
                &r.sender,
                &r.control_id,
                &format!(
                    "FT1[{}]-6 transaction type {:?} not supported (CG only)",
                    ft1.set_id, ft1.transaction_type
                ),
            );
        }
        match &shared_currency {
            None => shared_currency = Some(ft1.currency.clone()),
            Some(c) if c == &ft1.currency => {}
            Some(c) => {
                return ae(
                    &r.sender,
                    &r.control_id,
                    &format!(
                        "FT1[{}] currency {:?} differs from prior {:?}",
                        ft1.set_id, ft1.currency, c
                    ),
                );
            }
        }
        parsed.push(ft1);
    }
    let Some(currency_code) = shared_currency else {
        return ae(&r.sender, &r.control_id, "no FT1 segments");
    };
    let currency = match Iso4217::new(&currency_code) {
        Ok(c) => c,
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };

    let person_id = r.person_id;
    let audit_clone = r.audit.clone();
    let parsed_clone = parsed.clone();
    let count = parsed.len();
    let outcome = state
        .db
        .transaction::<_, (uuid::Uuid, usize), crate::Error>(|txn| {
            Box::pin(async move {
                use crate::db::repositories::billing::BillingRepository;
                let account_id =
                    match BillingRepository::find_open_account_for_patient(txn, person_id).await? {
                        Some(a) => a.id,
                        None => {
                            let acct = Account::new(person_id, currency.clone());
                            let saved = BillingRepository::create_account(txn, &acct).await?;
                            AuditLogRepository::log(
                                txn,
                                "account",
                                saved.id,
                                "open_via_hl7v2_p03",
                                None,
                                Some(serde_json::to_value(&saved).unwrap_or_default()),
                                &audit_clone,
                            )
                            .await?;
                            saved.id
                        }
                    };
                for ft1 in &parsed_clone {
                    let amount = ft1.amount.parse::<rust_decimal::Decimal>().map_err(|e| {
                        crate::Error::validation(format!("FT1[{}]-11.1 amount: {e}", ft1.set_id))
                    })?;
                    let money = Money::new(amount, currency.clone());
                    let mut charge =
                        Charge::new(account_id, ft1.code.clone(), ft1.description.clone(), money);
                    if let Some(at) = ft1.posted_at {
                        charge.posted_at = at;
                    }
                    let saved = BillingRepository::create_charge(txn, &charge).await?;
                    AuditLogRepository::log(
                        txn,
                        "charge",
                        saved.id,
                        "post_via_hl7v2_p03",
                        None,
                        Some(serde_json::to_value(&saved).unwrap_or_default()),
                        &audit_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "ChargePosted",
                        &serde_json::json!({
                            "charge_id": saved.id,
                            "account_id": account_id,
                            "person_id": person_id,
                            "source": "hl7v2_p03",
                        }),
                    )
                    .await?;
                }
                Ok((account_id, count))
            })
        })
        .await;
    match outcome {
        Ok((account_id, n)) => {
            let diag = if n == 1 {
                format!("charge posted account={account_id}")
            } else {
                format!("charges_posted={n} account={account_id}")
            };
            aa(&r.sender, &r.control_id, Some(&diag))
        }
        Err(e) => ae(
            &r.sender,
            &r.control_id,
            &crate::Error::from(e).client_message(),
        ),
    }
}

// ─── /api/hl7/v2/schedule-book (SIU^S12) ───────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-book",
    tag = "Hl7",
    summary = "Schedule book",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn schedule_book(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let sch = match r.message.segment("SCH") {
        Some(s) => parse_sch(s),
        None => return ae(&r.sender, &r.control_id, "SCH segment missing"),
    };
    let Some(start) = sch.start else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-11 (start datetime) is required",
        );
    };
    let duration = sch.duration_minutes.unwrap_or(30);
    let end = start + chrono::Duration::minutes(i64::from(duration));

    let person_id = r.person_id;
    let audit_clone = r.audit.clone();
    let result = state
        .db
        .transaction::<_, Appointment, crate::Error>(|txn| {
            Box::pin(async move {
                use crate::db::repositories::appointment::AppointmentRepository;
                let overlapping =
                    AppointmentRepository::find_overlapping_for_patient(txn, person_id, start, end)
                        .await?;
                if !overlapping.is_empty() {
                    return Err(crate::Error::conflict(format!(
                        "appointment overlap for person {person_id} at {start}"
                    )));
                }
                let mut appt = Appointment::new(person_id, start, end);
                appt.status = AppointmentStatus::Booked;
                appt.reason = sch.reason.clone();
                let saved = AppointmentRepository::create(txn, &appt).await?;
                AuditLogRepository::log(
                    txn,
                    "appointment",
                    saved.id,
                    "book_via_hl7v2_s12",
                    None,
                    Some(serde_json::to_value(&saved).unwrap_or_default()),
                    &audit_clone,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "AppointmentBooked",
                    &serde_json::json!({
                        "appointment_id": saved.id,
                        "person_id": person_id,
                        "source": "hl7v2_s12",
                    }),
                )
                .await?;
                Ok(saved)
            })
        })
        .await;
    match result {
        Ok(a) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("filler={} person={person_id}", a.id)),
        ),
        Err(e) => ae(
            &r.sender,
            &r.control_id,
            &crate::Error::from(e).client_message(),
        ),
    }
}

// ─── /api/hl7/v2/schedule-reschedule (SIU^S13) ─────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-reschedule",
    tag = "Hl7",
    summary = "Schedule reschedule",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn schedule_reschedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let sch = match r.message.segment("SCH") {
        Some(s) => parse_sch(s),
        None => return ae(&r.sender, &r.control_id, "SCH segment missing"),
    };
    let Some(filler) = sch.filler_id else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-2 (filler appointment id) is required",
        );
    };
    let Some(start) = sch.start else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-11 (start datetime) is required",
        );
    };
    let duration = sch.duration_minutes.unwrap_or(30);
    let end = start + chrono::Duration::minutes(i64::from(duration));
    use crate::db::repositories::appointment::AppointmentRepository;
    let existing = match AppointmentRepository::find_by_id(&state.db, filler).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("filler {filler} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    if matches!(
        existing.status,
        AppointmentStatus::Cancelled | AppointmentStatus::NoShow | AppointmentStatus::Fulfilled
    ) {
        return ae(
            &r.sender,
            &r.control_id,
            &format!("appointment {filler} is terminal ({:?})", existing.status),
        );
    }
    let conflicts = match AppointmentRepository::find_overlapping_for_patient_excluding(
        &state.db,
        existing.person_id,
        start,
        end,
        filler,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    if !conflicts.is_empty() {
        return ae(
            &r.sender,
            &r.control_id,
            &format!("overlap with appointment {}", conflicts[0].id),
        );
    }
    let mut updated = existing.clone();
    updated.start_datetime = start;
    updated.end_datetime = end;
    updated.updated_at = chrono::Utc::now();
    let saved = match AppointmentRepository::update(&state.db, &updated).await {
        Ok(a) => a,
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "appointment",
        saved.id,
        "reschedule_via_hl7v2_s13",
        Some(serde_json::to_value(&existing).unwrap_or_default()),
        Some(serde_json::to_value(&saved).unwrap_or_default()),
        &r.audit,
    )
    .await;
    let _ = OutboxRepository::publish(
        &state.db,
        "AppointmentRescheduled",
        &serde_json::json!({ "appointment_id": saved.id, "source": "hl7v2_s13" }),
    )
    .await;
    aa(
        &r.sender,
        &r.control_id,
        Some(&format!("rescheduled filler={filler}")),
    )
}

// ─── /api/hl7/v2/schedule-modify (SIU^S14) ─────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-modify",
    tag = "Hl7",
    summary = "Schedule modify",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn schedule_modify(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let sch = match r.message.segment("SCH") {
        Some(s) => parse_sch(s),
        None => return ae(&r.sender, &r.control_id, "SCH segment missing"),
    };
    let Some(filler) = sch.filler_id else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-2 (filler appointment id) is required",
        );
    };
    use crate::db::repositories::appointment::AppointmentRepository;
    let existing = match AppointmentRepository::find_by_id(&state.db, filler).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("filler {filler} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    if matches!(
        existing.status,
        AppointmentStatus::Cancelled | AppointmentStatus::NoShow | AppointmentStatus::Fulfilled
    ) {
        return ae(
            &r.sender,
            &r.control_id,
            &format!("appointment {filler} is terminal ({:?})", existing.status),
        );
    }
    let mut updated = existing.clone();
    updated.reason = sch.reason.clone();
    updated.updated_at = chrono::Utc::now();
    let saved = match AppointmentRepository::update(&state.db, &updated).await {
        Ok(a) => a,
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    let _ = AuditLogRepository::log(
        &state.db,
        "appointment",
        saved.id,
        "modify_via_hl7v2_s14",
        Some(serde_json::to_value(&existing).unwrap_or_default()),
        Some(serde_json::to_value(&saved).unwrap_or_default()),
        &r.audit,
    )
    .await;
    let _ = OutboxRepository::publish(
        &state.db,
        "AppointmentModified",
        &serde_json::json!({ "appointment_id": saved.id, "source": "hl7v2_s14" }),
    )
    .await;
    aa(
        &r.sender,
        &r.control_id,
        Some(&format!("modified filler={filler}")),
    )
}

// ─── /api/hl7/v2/schedule-cancel (SIU^S15) ─────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-cancel",
    tag = "Hl7",
    summary = "Schedule cancel",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn schedule_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let sch = match r.message.segment("SCH") {
        Some(s) => parse_sch(s),
        None => return ae(&r.sender, &r.control_id, "SCH segment missing"),
    };
    let Some(filler) = sch.filler_id else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-2 (filler appointment id) is required",
        );
    };
    match state
        .scheduling
        .cancel(filler, CancellationReason::PatientRequest, &r.audit)
        .await
    {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("cancelled filler={filler}")),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

// ─── /api/hl7/v2/mfn-staff (MFN^M02) ───────────────────────────────────

fn need_worker_client(state: &AppState) -> Option<&dyn WorkerClient> {
    state.worker_client.as_deref()
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/mfn-staff",
    tag = "Hl7",
    summary = "Mfn staff",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn mfn_staff(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let mfi = match msg.segment("MFI") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "MFI segment missing"),
    };
    let file_id = mfi.raw(1).trim().to_ascii_uppercase();
    if file_id != "PRA" {
        return ae(
            &h.sender,
            &h.control_id,
            &format!("MFI-1 file id {file_id:?} not supported (PRA only)"),
        );
    }
    let Some(client) = need_worker_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "WORKER_SERVICE_URL unset; cannot apply staff master file",
        );
    };
    let audit = request_audit_ctx(&headers);
    let up = upstream_ctx(&audit);
    let mut items: Vec<crate::hl7v2::pid_translation::StaffItem> = Vec::new();
    let mut current_mfe: Option<&crate::hl7v2::Segment> = None;
    for seg in &msg.segments {
        match seg.name.as_str() {
            "MFE" => current_mfe = Some(seg),
            "STF" => {
                if let Some(mfe) = current_mfe.take() {
                    match parse_mfe_stf(mfe, seg) {
                        Ok(it) => items.push(it),
                        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
                    }
                } else {
                    return ae(&h.sender, &h.control_id, "STF without preceding MFE");
                }
            }
            _ => {}
        }
    }
    if items.is_empty() {
        return ae(&h.sender, &h.control_id, "no MFE+STF pairs in message");
    }
    let mut applied = 0usize;
    for item in &items {
        let hits = match client.search(&item.staff_id, 20, &up).await {
            Ok(h) => h,
            Err(e) => {
                return ae(
                    &h.sender,
                    &h.control_id,
                    &format!("upstream search: {}", client_text(&e)),
                );
            }
        };
        let existing = hits
            .into_iter()
            .find(|w| worker_has_identifier(w, &item.staff_id));
        let body = staff_item_to_worker_create(item);
        match item.event_code.as_str() {
            "MAD" => {
                if existing.is_some() {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("MAD: staff id {:?} already exists", item.staff_id),
                    );
                }
                if let Err(e) = client.create(body, &up).await {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("upstream create: {}", client_text(&e)),
                    );
                }
                applied += 1;
            }
            "MUP" => {
                let Some(w) = existing else {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("MUP: staff id {:?} not found", item.staff_id),
                    );
                };
                if let Err(e) = client.update(w.id, body, &up).await {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("upstream update: {}", client_text(&e)),
                    );
                }
                applied += 1;
            }
            "MDL" => {
                let Some(w) = existing else {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("MDL: staff id {:?} not found", item.staff_id),
                    );
                };
                if let Err(e) = client.delete(w.id, &up).await {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("upstream delete: {}", client_text(&e)),
                    );
                }
                applied += 1;
            }
            _ => unreachable!(),
        }
    }
    aa(
        &h.sender,
        &h.control_id,
        Some(&format!("staff_records_applied={applied}")),
    )
}

fn worker_has_identifier(w: &crate::upstream::Worker, target: &str) -> bool {
    let Some(arr) = w.data.get("identifiers").and_then(|v| v.as_array()) else {
        return false;
    };
    arr.iter().any(|id| {
        id.get("value")
            .and_then(|v| v.as_str())
            .map(|s| s == target)
            .unwrap_or(false)
    })
}

fn staff_item_to_worker_create(item: &crate::hl7v2::pid_translation::StaffItem) -> WorkerCreate {
    let mut fields = serde_json::Map::new();
    let id_obj = serde_json::json!({
        "use": "official",
        "type": "Other",
        "system": "urn:hl7v2:staff:id",
        "value": item.staff_id,
    });
    fields.insert("identifiers".into(), serde_json::Value::Array(vec![id_obj]));
    let mut name = serde_json::Map::new();
    if !item.family.is_empty() {
        name.insert(
            "family".into(),
            serde_json::Value::String(item.family.clone()),
        );
    }
    let mut given = Vec::new();
    if !item.given.is_empty() {
        given.push(serde_json::Value::String(item.given.clone()));
    }
    if let Some(m) = &item.middle {
        given.push(serde_json::Value::String(m.clone()));
    }
    if !given.is_empty() {
        name.insert("given".into(), serde_json::Value::Array(given));
    }
    if !name.is_empty() {
        fields.insert("name".into(), serde_json::Value::Object(name));
    }
    if let Some(g) = &item.gender {
        fields.insert("gender".into(), serde_json::Value::String(g.clone()));
    }
    if let Some(bd) = &item.birth_date {
        fields.insert("birth_date".into(), serde_json::Value::String(bd.clone()));
    }
    fields.insert("active".into(), serde_json::Value::Bool(item.active));
    WorkerCreate { fields }
}

// ─── /api/hl7/v2/mfn-location (MFN^M05) ────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/hl7/v2/mfn-location",
    tag = "Hl7",
    summary = "Mfn location",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn mfn_location(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let mfi = match msg.segment("MFI") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "MFI segment missing"),
    };
    let file_id = mfi.raw(1).trim().to_ascii_uppercase();
    if file_id != "LOC" {
        return ae(
            &h.sender,
            &h.control_id,
            &format!("MFI-1 file id {file_id:?} not supported (LOC only)"),
        );
    }
    let audit = request_audit_ctx(&headers);
    let mut items: Vec<crate::hl7v2::pid_translation::LocationItem> = Vec::new();
    let mut current_mfe: Option<&crate::hl7v2::Segment> = None;
    for seg in &msg.segments {
        match seg.name.as_str() {
            "MFE" => current_mfe = Some(seg),
            "LOC" => {
                if let Some(mfe) = current_mfe.take() {
                    match parse_mfe_loc(mfe, seg) {
                        Ok(it) => items.push(it),
                        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
                    }
                } else {
                    return ae(&h.sender, &h.control_id, "LOC without preceding MFE");
                }
            }
            _ => {}
        }
    }
    if items.is_empty() {
        return ae(&h.sender, &h.control_id, "no MFE+LOC pairs in message");
    }
    use crate::db::repositories::bed::BedRepository;
    use crate::db::repositories::facility::RoomRepository;
    use crate::models::facility::Bed;
    let audit_clone = audit.clone();
    let items_clone = items.clone();
    let result = state
        .db
        .transaction::<_, usize, crate::Error>(|txn| {
            Box::pin(async move {
                let mut applied = 0usize;
                for item in &items_clone {
                    let existing = BedRepository::find_by_code(txn, &item.bed_code).await?;
                    match item.event_code.as_str() {
                        "MAD" => {
                            if existing.is_some() {
                                return Err(crate::Error::conflict(format!(
                                    "MAD: bed code {:?} already exists",
                                    item.bed_code
                                )));
                            }
                            let room_code = item.room_code.as_deref().ok_or_else(|| {
                                crate::Error::validation(
                                    "MFN^M05 MAD: LOC-1.2 (room code) is required to attach a new bed",
                                )
                            })?;
                            let room = RoomRepository::find_by_code(txn, room_code)
                                .await?
                                .ok_or_else(|| {
                                    crate::Error::not_found(format!(
                                        "MFN^M05 MAD: room code {room_code:?} not found"
                                    ))
                                })?;
                            let bed_name = item
                                .bed_name
                                .clone()
                                .unwrap_or_else(|| item.bed_code.clone());
                            let bed = Bed::new(room.id, bed_name, item.bed_code.clone());
                            let created = BedRepository::create(txn, &bed).await?;
                            AuditLogRepository::log(
                                txn,
                                "bed",
                                created.id,
                                "mad_via_hl7v2_mfn_m05",
                                None,
                                Some(serde_json::to_value(&created).unwrap_or_default()),
                                &audit_clone,
                            )
                            .await?;
                            applied += 1;
                        }
                        "MUP" => {
                            let Some(mut bed) = existing else {
                                return Err(crate::Error::not_found(format!(
                                    "MUP: bed code {:?} not found",
                                    item.bed_code
                                )));
                            };
                            if let Some(name) = &item.bed_name {
                                bed.name = name.clone();
                            }
                            bed.updated_at = chrono::Utc::now();
                            let updated = BedRepository::update(txn, &bed).await?;
                            AuditLogRepository::log(
                                txn,
                                "bed",
                                updated.id,
                                "mup_via_hl7v2_mfn_m05",
                                None,
                                Some(serde_json::to_value(&updated).unwrap_or_default()),
                                &audit_clone,
                            )
                            .await?;
                            applied += 1;
                        }
                        "MDL" => {
                            let Some(bed) = existing else {
                                return Err(crate::Error::not_found(format!(
                                    "MDL: bed code {:?} not found",
                                    item.bed_code
                                )));
                            };
                            BedRepository::set_status_unchecked(
                                txn,
                                bed.id,
                                BedStatus::OutOfService,
                            )
                            .await?;
                            AuditLogRepository::log(
                                txn,
                                "bed",
                                bed.id,
                                "mdl_via_hl7v2_mfn_m05",
                                None,
                                Some(serde_json::json!({ "bed_code": item.bed_code })),
                                &audit_clone,
                            )
                            .await?;
                            applied += 1;
                        }
                        _ => unreachable!(),
                    }
                }
                Ok(applied)
            })
        })
        .await;
    match result {
        Ok(n) => aa(
            &h.sender,
            &h.control_id,
            Some(&format!("location_records_applied={n}")),
        ),
        Err(e) => ae(
            &h.sender,
            &h.control_id,
            &crate::Error::from(e).client_message(),
        ),
    }
}

// ─── /api/hl7/v2/batch (FHS/BHS envelope dispatcher) ───────────────────

async fn dispatch_inner_to_ack_body(
    state: &AppState,
    headers: &HeaderMap,
    msg: &crate::hl7v2::Message,
) -> String {
    let body = encode_message(msg);
    let msh = msg.segment("MSH");
    let (code, event) = match msh {
        Some(s) => {
            let field = s.raw(9);
            let mut parts = field.split('^');
            (
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            )
        }
        None => (String::new(), String::new()),
    };
    macro_rules! call {
        ($handler:ident) => {{
            let resp = $handler(State(state.clone()), headers.clone(), body.clone())
                .await
                .into_response();
            let (_, body_box) = resp.into_parts();
            let bytes = axum::body::to_bytes(body_box, 1024 * 1024)
                .await
                .unwrap_or_default();
            String::from_utf8(bytes.to_vec()).unwrap_or_default()
        }};
    }
    match (code.as_str(), event.as_str()) {
        ("ADT", "A01") => call!(admit),
        ("ADT", "A02") => call!(transfer),
        ("ADT", "A03") => call!(discharge),
        ("ADT", "A04") => call!(register),
        ("ADT", "A05") => call!(pre_admit),
        ("ADT", "A06") => call!(change_to_inpatient),
        ("ADT", "A07") => call!(change_to_outpatient),
        ("ADT", "A08") => call!(update),
        ("ADT", "A11") => call!(cancel_admit),
        ("ADT", "A12") => call!(cancel_transfer),
        ("ADT", "A13") => call!(cancel_discharge),
        ("ADT", "A21") => call!(leave_start),
        ("ADT", "A22") => call!(leave_end),
        ("ADT", "A23") => call!(delete_patient),
        ("ADT", "A28") => call!(add_person),
        ("ADT", "A38") => call!(cancel_pre_admit),
        ("ADT", "A40") => call!(merge),
        ("DFT", "P03") => call!(dft),
        ("SIU", "S12") => call!(schedule_book),
        ("SIU", "S13") => call!(schedule_reschedule),
        ("SIU", "S14") => call!(schedule_modify),
        ("SIU", "S15") => call!(schedule_cancel),
        ("MFN", "M02") => call!(mfn_staff),
        ("MFN", "M05") => call!(mfn_location),
        _ => call!(add_person),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/batch",
    tag = "Hl7",
    summary = "Batch",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let parsed = match parse_batch(&body) {
        Ok(b) => b,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let batch_id = parsed
        .bhs
        .as_ref()
        .map(|s| s.raw(11).to_string())
        .unwrap_or_default();
    let sender = parsed
        .bhs
        .as_ref()
        .map(|s| s.raw(3).to_string())
        .unwrap_or_default();
    let mut per_message: Vec<String> = Vec::with_capacity(parsed.messages.len());
    for inner in &parsed.messages {
        per_message.push(dispatch_inner_to_ack_body(&state, &headers, inner).await);
    }
    let envelope = encode_batch_ack(PAS_APP, &sender, &batch_id, &per_message);
    (StatusCode::OK, TEXT_PLAIN, envelope)
}
