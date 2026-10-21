//! REST proxy handlers for the v0.44.0 Option C identity + workforce
//! delegation.
//!
//! Every `/api/patients/*` route here proxies to `person-service` via
//! [`crate::upstream::PersonClient`]. Every `/api/practitioners/*`
//! route proxies to `worker-service` via
//! [`crate::upstream::WorkerClient`]. Inbound merge webhooks land at
//! `/api/upstream/{person,worker}-merged` and propagate the upstream's
//! merge decision into PAS-owned FK columns.
//!
//! ## Wire-shape note
//!
//! Phase 3a does **transparent proxy**: the upstream's `Person` /
//! `Worker` JSON shape is forwarded under the PAS `ApiResponse` envelope
//! verbatim. The v0.43.0 PAS `Patient` JSON shape is **not** preserved;
//! callers see whatever `person-service` returns. This is a
//! deliberate Option C trade-off — see `spec.md` §4.0 wire-shape note.
//!
//! ## User context
//!
//! Every outbound upstream call attaches `X-User-Id` / `X-User-Ip` /
//! `X-User-Agent` headers extracted from the inbound request, so the
//! upstream's audit log credits the original PAS-side caller. Headers
//! that are absent from the inbound request are not attached.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::upstream::{
    MergeOutcome, PersonClient, PersonCreate, UserContext, WorkerClient, WorkerCreate,
};

// ─── User-context extractor ─────────────────────────────────────────────

/// Build an upstream [`UserContext`] from inbound `X-User-*` headers.
/// Every header is optional; missing headers leave the corresponding
/// field as `None`.
pub fn user_context_from_headers(headers: &HeaderMap) -> UserContext {
    let take = |name: &str| -> Option<String> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(std::string::ToString::to_string)
    };
    UserContext {
        user_id: take("x-user-id"),
        user_ip: take("x-user-ip"),
        user_agent: take("x-user-agent"),
    }
}

// ─── Shared helpers ─────────────────────────────────────────────────────

fn ok_value(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

fn need_person_client(state: &AppState) -> Result<&dyn PersonClient, Error> {
    state
        .person_client
        .as_deref()
        .ok_or_else(|| Error::upstream("PERSON_SERVICE_URL unset; identity proxy disabled"))
}

fn need_worker_client(state: &AppState) -> Result<&dyn WorkerClient, Error> {
    state
        .worker_client
        .as_deref()
        .ok_or_else(|| Error::upstream("WORKER_SERVICE_URL unset; workforce proxy disabled"))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

// ─── /api/patients/* (proxy person-service) ────────────────────────

pub async fn create_patient(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let create = PersonCreate {
        fields: match body {
            Value::Object(m) => m,
            other => {
                return error_to_response(Error::validation(format!(
                    "POST /api/patients body must be a JSON object, got {other:?}"
                )));
            }
        },
    };
    // An NHS number that fails its check digit never reaches person-service (DTAC-8).
    if let Err(e) = crate::validation::check_person_identifiers(&create.fields) {
        return error_to_response(e);
    }
    match client.create(create, &ctx).await {
        Ok(person) => ok_value(serde_json::to_value(person).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn get_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.get(id, &ctx).await {
        Ok(person) => ok_value(serde_json::to_value(person).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn update_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let update = PersonCreate {
        fields: match body {
            Value::Object(m) => m,
            other => {
                return error_to_response(Error::validation(format!(
                    "PUT /api/patients/{{id}} body must be a JSON object, got {other:?}"
                )));
            }
        },
    };
    if let Err(e) = crate::validation::check_person_identifiers(&update.fields) {
        return error_to_response(e);
    }
    match client.update(id, update, &ctx).await {
        Ok(person) => ok_value(serde_json::to_value(person).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn delete_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.delete(id, &ctx).await {
        Ok(()) => ok_value(Value::Null),
        Err(e) => error_to_response(e),
    }
}

pub async fn search_patients(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let query = q.q.unwrap_or_default();
    match client.search(&query, q.limit, &ctx).await {
        Ok(persons) => ok_value(serde_json::to_value(persons).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn masked_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.masked(id, &ctx).await {
        Ok(person) => ok_value(serde_json::to_value(person).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn export_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    // Phase 3a returns just the upstream's GDPR export. Phase 3b will
    // additionally join PAS-owned related rows (encounters, appointments,
    // billing, consents) keyed by person_id.
    match client.export(id, &ctx).await {
        Ok(value) => ok_value(value),
        Err(e) => error_to_response(e),
    }
}

pub async fn audit_patient(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    // Phase 3a returns just upstream audit. Phase 3b will merge in
    // PAS-owned audit_log entries (admit/discharge/charge/letter/etc.)
    // time-sorted with the upstream entries.
    match client.audit(id, q.limit, &ctx).await {
        Ok(entries) => ok_value(serde_json::to_value(entries).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

// ─── /api/patients/export.{json,xml,tsv,csv} + /api/patients/import ────
//
// Phase 3d bulk interchange. The PAS no longer owns patient rows, so
// every bulk export/import becomes a transparent passthrough to
// `person-service`. We do NOT wrap the body in the PAS `ApiResponse`
// envelope here — callers feeding the output into their own tooling
// (CSV → Excel, XML → integration engine, etc.) want the raw bytes
// with the upstream's content-type.

async fn bulk_export_format(
    state: &AppState,
    headers: &HeaderMap,
    format: &str,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let client = match need_person_client(state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e).into_response(),
    };
    let ctx = user_context_from_headers(headers);
    match client.bulk_export(format, &ctx).await {
        Ok((content_type, body)) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, content_type)],
            body,
        )
            .into_response(),
        Err(e) => error_to_response(e).into_response(),
    }
}

pub async fn export_patients_json(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    bulk_export_format(&state, &headers, "json").await
}

pub async fn export_patients_xml(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    bulk_export_format(&state, &headers, "xml").await
}

pub async fn export_patients_tsv(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    bulk_export_format(&state, &headers, "tsv").await
}

pub async fn export_patients_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    bulk_export_format(&state, &headers, "csv").await
}

/// `POST /api/patients/import` — accepts any content-type the upstream
/// parses (`application/json`, `application/xml`, `text/csv`,
/// `text/tab-separated-values`) and forwards the body verbatim.
/// Returns the upstream's import report (as JSON) under the PAS
/// envelope so callers can integrate with the same error-handling
/// surface as the typed endpoints.
pub async fn import_patients(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> HandlerResult {
    let client = match need_person_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    match client.bulk_import(&content_type, body.to_vec(), &ctx).await {
        Ok(value) => ok_value(value),
        Err(e) => error_to_response(e),
    }
}

// ─── /api/practitioners/* (proxy worker-service) ───────────────────

pub async fn create_practitioner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let create = WorkerCreate {
        fields: match body {
            Value::Object(m) => m,
            other => {
                return error_to_response(Error::validation(format!(
                    "POST /api/practitioners body must be a JSON object, got {other:?}"
                )));
            }
        },
    };
    match client.create(create, &ctx).await {
        Ok(worker) => ok_value(serde_json::to_value(worker).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn get_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.get(id, &ctx).await {
        Ok(worker) => ok_value(serde_json::to_value(worker).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn update_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let update = WorkerCreate {
        fields: match body {
            Value::Object(m) => m,
            other => {
                return error_to_response(Error::validation(format!(
                    "PUT /api/practitioners/{{id}} body must be a JSON object, got {other:?}"
                )));
            }
        },
    };
    match client.update(id, update, &ctx).await {
        Ok(worker) => ok_value(serde_json::to_value(worker).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn delete_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.delete(id, &ctx).await {
        Ok(()) => ok_value(Value::Null),
        Err(e) => error_to_response(e),
    }
}

pub async fn search_practitioners(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    let query = q.q.unwrap_or_default();
    match client.search(&query, q.limit, &ctx).await {
        Ok(workers) => ok_value(serde_json::to_value(workers).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn masked_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.masked(id, &ctx).await {
        Ok(worker) => ok_value(serde_json::to_value(worker).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

pub async fn export_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.export(id, &ctx).await {
        Ok(value) => ok_value(value),
        Err(e) => error_to_response(e),
    }
}

pub async fn audit_practitioner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> HandlerResult {
    let client = match need_worker_client(&state) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    let ctx = user_context_from_headers(&headers);
    match client.audit(id, q.limit, &ctx).await {
        Ok(entries) => ok_value(serde_json::to_value(entries).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

// ─── Inbound merge webhooks ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MergeWebhookBody {
    pub source_id: Uuid,
    pub target_id: Uuid,
    #[serde(default)]
    pub event_id: Option<Uuid>,
}

const SIG_HEADER: &str = "x-upstream-signature";
const EVENT_ID_HEADER: &str = "x-upstream-event-id";

/// Verify HMAC-SHA256 signature `sha256=<hex>` over `body` keyed by `secret`.
/// Constant-time comparison via the `hmac::Mac::verify_slice` path.
fn verify_hmac(secret: &[u8], body: &[u8], header_value: &str) -> Result<(), Error> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let provided_hex = header_value
        .strip_prefix("sha256=")
        .ok_or_else(|| Error::upstream("X-Upstream-Signature must start with sha256="))?;
    let provided = hex_decode(provided_hex)
        .ok_or_else(|| Error::upstream("X-Upstream-Signature is not valid hex"))?;
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret)
        .map_err(|_| Error::config("HMAC secret is empty"))?;
    mac.update(body);
    mac.verify_slice(&provided)
        .map_err(|_| Error::upstream("HMAC signature mismatch"))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = (bytes[i] as char).to_digit(16)? as u8;
        let lo = (bytes[i + 1] as char).to_digit(16)? as u8;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

/// Check + record an inbound webhook event id. Returns
/// - `Ok(true)` — new event, side-effects should run
/// - `Ok(false)` — duplicate event, side-effects should be skipped
/// - `Err` — DB error
async fn record_inbox(
    db: &sea_orm::DatabaseConnection,
    event_id: Uuid,
    source: &str,
) -> Result<bool, Error> {
    use sea_orm::ConnectionTrait;
    let sql = sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO upstream_inbox (event_id, source) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        [event_id.into(), source.into()],
    );
    let result = db.execute_raw(sql).await.map_err(Error::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Rewrite every PAS-owned `person_id` column referencing the merge
/// source to point at the merge target, in one DB transaction. Logs one
/// `audit_log` row per affected table.
async fn propagate_person_merge(
    db: &sea_orm::DatabaseConnection,
    outcome: &MergeOutcome,
    ctx: &crate::db::repositories::audit::UserContext,
) -> Result<(), Error> {
    use sea_orm::TransactionTrait;
    let outcome = outcome.clone();
    let ctx = ctx.clone();
    db.transaction::<_, (), Error>(|txn| {
        Box::pin(async move {
            use crate::db::repositories::audit::AuditLogRepository;
            use sea_orm::ConnectionTrait;
            // Every table that carries person_id. Phase 3a does a single
            // bulk UPDATE per table; Phase 3b may optimize / batch.
            for table in [
                "encounters",
                "appointments",
                "appointment_series",
                "referrals",
                "waitlist_entries",
                "rtt_pathways",
                "generated_letters",
                "accounts",
                "consents",
                "coverages",
            ] {
                let sql = sea_orm::Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    format!("UPDATE {table} SET person_id = $1 WHERE person_id = $2"),
                    [outcome.target_id.into(), outcome.source_id.into()],
                );
                let result = txn.execute_raw(sql).await.map_err(Error::Database)?;
                if result.rows_affected() > 0 {
                    AuditLogRepository::log(
                        txn,
                        table,
                        outcome.source_id,
                        "upstream_person_merged",
                        Some(serde_json::json!({ "person_id": outcome.source_id })),
                        Some(serde_json::json!({
                            "person_id": outcome.target_id,
                            "rows_rewritten": result.rows_affected(),
                        })),
                        &ctx,
                    )
                    .await?;
                }
            }
            Ok(())
        })
    })
    .await
    .map_err(|e| match e {
        sea_orm::TransactionError::Connection(c) => Error::Database(c),
        sea_orm::TransactionError::Transaction(t) => t,
    })
}

async fn propagate_worker_merge(
    db: &sea_orm::DatabaseConnection,
    outcome: &MergeOutcome,
    ctx: &crate::db::repositories::audit::UserContext,
) -> Result<(), Error> {
    use sea_orm::TransactionTrait;
    let outcome = outcome.clone();
    let ctx = ctx.clone();
    db.transaction::<_, (), Error>(|txn| {
        Box::pin(async move {
            use crate::db::repositories::audit::AuditLogRepository;
            use sea_orm::ConnectionTrait;
            for (table, column) in [
                ("encounters", "worker_id"),
                ("appointments", "worker_id"),
                ("appointment_series", "worker_id"),
                ("admissions", "admitting_worker_id"),
                ("discharges", "discharging_worker_id"),
                ("referrals", "referring_worker_id"),
            ] {
                let sql = sea_orm::Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    format!("UPDATE {table} SET {column} = $1 WHERE {column} = $2"),
                    [outcome.target_id.into(), outcome.source_id.into()],
                );
                let result = txn.execute_raw(sql).await.map_err(Error::Database)?;
                if result.rows_affected() > 0 {
                    AuditLogRepository::log(
                        txn,
                        table,
                        outcome.source_id,
                        "upstream_worker_merged",
                        Some(serde_json::json!({ column: outcome.source_id })),
                        Some(serde_json::json!({
                            column: outcome.target_id,
                            "rows_rewritten": result.rows_affected(),
                        })),
                        &ctx,
                    )
                    .await?;
                }
            }
            Ok(())
        })
    })
    .await
    .map_err(|e| match e {
        sea_orm::TransactionError::Connection(c) => Error::Database(c),
        sea_orm::TransactionError::Transaction(t) => t,
    })
}

/// `POST /api/upstream/person-merged` — receive a merge notification from
/// `person-service`, propagate it to PAS-owned `person_id` columns.
pub async fn person_merged_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    raw: axum::body::Bytes,
) -> HandlerResult {
    // 1. Verify signature (config-gated). PERSON_SERVICE_WEBHOOK_SECRET
    //    must be set; otherwise every call refuses.
    let secret = match std::env::var("PERSON_SERVICE_WEBHOOK_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => {
            return error_to_response(Error::upstream(
                "PERSON_SERVICE_WEBHOOK_SECRET unset; refusing inbound webhook",
            ));
        }
    };
    let sig = headers
        .get(SIG_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Err(e) = verify_hmac(secret.as_bytes(), &raw, sig) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<Value>::err("UNSIGNED_WEBHOOK", e.to_string())),
        );
    }

    // 2. Parse body.
    let body: MergeWebhookBody = match serde_json::from_slice(&raw) {
        Ok(b) => b,
        Err(e) => {
            return error_to_response(Error::validation(format!("malformed webhook body: {e}")));
        }
    };
    if body.source_id == body.target_id {
        return error_to_response(Error::validation("self-merge: source_id == target_id"));
    }

    // 3. Dedupe via upstream_inbox.
    let event_id = body.event_id.unwrap_or_else(|| {
        // Best-effort: when no event_id header / body field is supplied,
        // synthesize a deterministic id from the merge pair so a retry
        // with the same payload is still treated as a duplicate.
        Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{}|{}", body.source_id, body.target_id).as_bytes(),
        )
    });
    let event_id = headers
        .get(EVENT_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or(event_id);
    let is_new = match record_inbox(&state.db, event_id, "person_service").await {
        Ok(b) => b,
        Err(e) => return error_to_response(e),
    };
    if !is_new {
        // Replay — ACK 200 but skip side-effects.
        return ok_value(serde_json::json!({
            "event_id": event_id,
            "applied": false,
            "reason": "duplicate"
        }));
    }

    // 4. Propagate to PAS-owned rows.
    let audit_ctx = crate::db::repositories::audit::UserContext {
        user_id: Some("person-service".to_string()),
        user_ip: None,
        user_agent: Some(format!("upstream-webhook event={event_id}")),
    };
    let outcome = MergeOutcome {
        source_id: body.source_id,
        target_id: body.target_id,
    };
    if let Err(e) = propagate_person_merge(&state.db, &outcome, &audit_ctx).await {
        return error_to_response(e);
    }
    ok_value(serde_json::json!({
        "event_id": event_id,
        "applied": true,
        "source_id": body.source_id,
        "target_id": body.target_id,
    }))
}

/// `POST /api/upstream/worker-merged` — same shape as person-merged but
/// for `worker-service` merges.
pub async fn worker_merged_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    raw: axum::body::Bytes,
) -> HandlerResult {
    let secret = match std::env::var("WORKER_SERVICE_WEBHOOK_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => {
            return error_to_response(Error::upstream(
                "WORKER_SERVICE_WEBHOOK_SECRET unset; refusing inbound webhook",
            ));
        }
    };
    let sig = headers
        .get(SIG_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Err(e) = verify_hmac(secret.as_bytes(), &raw, sig) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<Value>::err("UNSIGNED_WEBHOOK", e.to_string())),
        );
    }
    let body: MergeWebhookBody = match serde_json::from_slice(&raw) {
        Ok(b) => b,
        Err(e) => {
            return error_to_response(Error::validation(format!("malformed webhook body: {e}")));
        }
    };
    if body.source_id == body.target_id {
        return error_to_response(Error::validation("self-merge: source_id == target_id"));
    }
    let event_id = body.event_id.unwrap_or_else(|| {
        Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{}|{}", body.source_id, body.target_id).as_bytes(),
        )
    });
    let event_id = headers
        .get(EVENT_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or(event_id);
    let is_new = match record_inbox(&state.db, event_id, "worker_service").await {
        Ok(b) => b,
        Err(e) => return error_to_response(e),
    };
    if !is_new {
        return ok_value(serde_json::json!({
            "event_id": event_id,
            "applied": false,
            "reason": "duplicate"
        }));
    }
    let audit_ctx = crate::db::repositories::audit::UserContext {
        user_id: Some("worker-service".to_string()),
        user_ip: None,
        user_agent: Some(format!("upstream-webhook event={event_id}")),
    };
    let outcome = MergeOutcome {
        source_id: body.source_id,
        target_id: body.target_id,
    };
    if let Err(e) = propagate_worker_merge(&state.db, &outcome, &audit_ctx).await {
        return error_to_response(e);
    }
    ok_value(serde_json::json!({
        "event_id": event_id,
        "applied": true,
        "source_id": body.source_id,
        "target_id": body.target_id,
    }))
}

// ─── Unit tests for the small helpers ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn test_user_context_extracts_present_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-user-id", HeaderValue::from_static("alice"));
        headers.insert("x-user-ip", HeaderValue::from_static("10.0.0.1"));
        headers.insert("x-user-agent", HeaderValue::from_static("curl/8"));
        let ctx = user_context_from_headers(&headers);
        assert_eq!(ctx.user_id.as_deref(), Some("alice"));
        assert_eq!(ctx.user_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(ctx.user_agent.as_deref(), Some("curl/8"));
    }

    #[test]
    fn test_user_context_missing_headers_are_none() {
        let ctx = user_context_from_headers(&HeaderMap::new());
        assert!(ctx.user_id.is_none());
        assert!(ctx.user_ip.is_none());
        assert!(ctx.user_agent.is_none());
    }

    #[test]
    fn test_hex_decode_lowercase() {
        assert_eq!(hex_decode("abcd"), Some(vec![0xab, 0xcd]));
    }

    #[test]
    fn test_hex_decode_uppercase() {
        assert_eq!(hex_decode("ABCD"), Some(vec![0xab, 0xcd]));
    }

    #[test]
    fn test_hex_decode_odd_length_rejected() {
        assert_eq!(hex_decode("abc"), None);
    }

    #[test]
    fn test_hex_decode_non_hex_char_rejected() {
        assert_eq!(hex_decode("zzzz"), None);
    }

    #[test]
    fn test_verify_hmac_accepts_correct_signature() {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        let secret = b"my-secret";
        let body = b"hello world";
        let mut mac = <Hmac<Sha256>>::new_from_slice(secret).unwrap();
        mac.update(body);
        let sig = format!(
            "sha256={}",
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert!(verify_hmac(secret, body, &sig).is_ok());
    }

    #[test]
    fn test_verify_hmac_rejects_wrong_secret() {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        let mut mac = <Hmac<Sha256>>::new_from_slice(b"other").unwrap();
        mac.update(b"hello world");
        let sig = format!(
            "sha256={}",
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert!(verify_hmac(b"my-secret", b"hello world", &sig).is_err());
    }

    #[test]
    fn test_verify_hmac_rejects_missing_prefix() {
        assert!(verify_hmac(b"k", b"v", "abcd").is_err());
    }
}
