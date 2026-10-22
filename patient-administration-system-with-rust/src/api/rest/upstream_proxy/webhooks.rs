//! Part of [`super`]: webhooks.

use super::*;

#[derive(Debug, Deserialize)]
pub struct MergeWebhookBody {
    pub source_id: Uuid,
    pub target_id: Uuid,
    #[serde(default)]
    pub event_id: Option<Uuid>,
}

pub(super) const SIG_HEADER: &str = "x-upstream-signature";

pub(super) const EVENT_ID_HEADER: &str = "x-upstream-event-id";

/// Verify HMAC-SHA256 signature `sha256=<hex>` over `body` keyed by `secret`.
/// Constant-time comparison via the `hmac::Mac::verify_slice` path.
pub(super) fn verify_hmac(secret: &[u8], body: &[u8], header_value: &str) -> Result<(), Error> {
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

pub(super) fn hex_decode(s: &str) -> Option<Vec<u8>> {
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
pub(super) async fn record_inbox(
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
pub(super) async fn propagate_person_merge(
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
            // Every table that carries person_id: one bulk UPDATE per table.
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
    .map_err(Error::from)
}

pub(super) async fn propagate_worker_merge(
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
    .map_err(Error::from)
}

/// `POST /api/upstream/person-merged` — receive a merge notification from
/// `person-service`, propagate it to PAS-owned `person_id` columns.
#[utoipa::path(
    post,
    path = "/api/upstream/person-merged",
    tag = "Upstream",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn person_merged_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    raw: axum::body::Bytes,
) -> HandlerResult {
    // 1. Verify signature (config-gated). PERSON_SERVICE_WEBHOOK_SECRET
    //    must be set; otherwise every call refuses.
    let secret = match crate::config::env::text("PERSON_SERVICE_WEBHOOK_SECRET") {
        Some(s) => s,
        None => {
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
        return ok(serde_json::json!({
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
    ok(serde_json::json!({
        "event_id": event_id,
        "applied": true,
        "source_id": body.source_id,
        "target_id": body.target_id,
    }))
}

/// `POST /api/upstream/worker-merged` — same shape as person-merged but
/// for `worker-service` merges.
#[utoipa::path(
    post,
    path = "/api/upstream/worker-merged",
    tag = "Upstream",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn worker_merged_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    raw: axum::body::Bytes,
) -> HandlerResult {
    let secret = match crate::config::env::text("WORKER_SERVICE_WEBHOOK_SECRET") {
        Some(s) => s,
        None => {
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
        return ok(serde_json::json!({
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
    ok(serde_json::json!({
        "event_id": event_id,
        "applied": true,
        "source_id": body.source_id,
        "target_id": body.target_id,
    }))
}
