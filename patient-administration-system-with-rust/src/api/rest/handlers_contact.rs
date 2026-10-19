//! REST handlers for contact reliability and communication needs (CAP-4).

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
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::contact::{DEFAULT_STALE_DAYS, ProfileInput};
use crate::db::repositories::audit::UserContext as RepoUserContext;

fn audit_ctx(headers: &HeaderMap) -> RepoUserContext {
    let uc = user_context_from_headers(headers);
    RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    }
}

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `GET /api/patients/{id}/communication-profile`: how to reach this patient well. Includes a
/// `fhir` Patient fragment for systems that speak FHIR.
pub async fn get_profile(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.contact.get(id).await {
        Ok(Some(p)) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!(
            "no communication profile for patient {id}"
        ))),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/patients/{id}/communication-profile`: create or replace. Changing the mobile number
/// clears its verification.
pub async fn put_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<ProfileInput>,
) -> HandlerResult {
    match state.contact.put(id, input, &audit_ctx(&headers)).await {
        Ok(p) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct VerifyBody {
    /// `staff`, `portal` or `sms_code`.
    pub method: String,
}

/// `POST /api/patients/{id}/communication-profile/verify-mobile`: record that the number was checked.
pub async fn verify_mobile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<VerifyBody>,
) -> HandlerResult {
    match state
        .contact
        .verify_mobile(id, &body.method, &audit_ctx(&headers))
        .await
    {
        Ok(p) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct StaleQuery {
    pub older_than_days: Option<i64>,
    pub limit: Option<i64>,
}

/// `GET /api/communication-profiles/stale?older_than_days=365&limit=100`: profiles whose mobile is
/// missing, unverified or old, oldest first.
pub async fn stale_profiles(
    State(state): State<AppState>,
    Query(q): Query<StaleQuery>,
) -> HandlerResult {
    match state
        .contact
        .stale(
            q.older_than_days.unwrap_or(DEFAULT_STALE_DAYS),
            q.limit.unwrap_or(100),
        )
        .await
    {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Deserialize)]
pub struct OptOutBody {
    /// `sms`, `whatsapp`, `apple_messages` or `email`.
    pub channel: String,
}

const OPTOUT_CHANNELS: [&str; 4] = ["sms", "whatsapp", "apple_messages", "email"];

fn check_channel(c: &str) -> Result<(), Error> {
    if OPTOUT_CHANNELS.contains(&c) {
        Ok(())
    } else {
        Err(Error::validation(
            "channel must be sms, whatsapp, apple_messages or email",
        ))
    }
}

/// `GET /api/patients/{id}/channel-optouts`: the channels this patient has asked us not to use.
pub async fn list_optouts(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    let rows = state
        .db
        .query_all(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT channel, opted_out_at, source FROM message_optouts WHERE subject = $1 AND resumed_at IS NULL ORDER BY channel",
            [crate::communication::consent::person_subject(id).into()],
        ))
        .await;
    match rows {
        Ok(rows) => {
            let list: Vec<Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "channel": r.try_get::<String>("", "channel").unwrap_or_default(),
                        "opted_out_at": r.try_get::<chrono::DateTime<chrono::Utc>>("", "opted_out_at").ok(),
                        "source": r.try_get::<String>("", "source").unwrap_or_default(),
                    })
                })
                .collect();
            ok(Value::Array(list))
        }
        Err(e) => error_to_response(Error::Database(e)),
    }
}

/// `POST /api/patients/{id}/channel-optouts`: staff record that the patient asked us to stop using a
/// channel (said on the phone, written, or through the portal). Takes effect at once.
pub async fn add_optout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<OptOutBody>,
) -> HandlerResult {
    if let Err(e) = check_channel(&body.channel) {
        return error_to_response(e);
    }
    let ctx = audit_ctx(&headers);
    match crate::communication::consent::record_optout(
        &state.db,
        &crate::communication::consent::person_subject(id),
        &body.channel,
        "staff",
        ctx.user_id.as_deref(),
    )
    .await
    {
        Ok(()) => ok(serde_json::json!({ "channel": body.channel, "opted_out": true })),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/patients/{id}/channel-optouts/{channel}`: the patient asked to hear from us again.
pub async fn remove_optout(
    State(state): State<AppState>,
    Path((id, channel)): Path<(Uuid, String)>,
) -> HandlerResult {
    if let Err(e) = check_channel(&channel) {
        return error_to_response(e);
    }
    match crate::communication::consent::resume(
        &state.db,
        &crate::communication::consent::person_subject(id),
        &channel,
    )
    .await
    {
        Ok(true) => ok(serde_json::json!({ "channel": channel, "opted_out": false })),
        Ok(false) => error_to_response(Error::not_found(
            "that patient has not opted out of that channel",
        )),
        Err(e) => error_to_response(e),
    }
}
