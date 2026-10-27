//! REST handlers for contact reliability and communication needs (CAP-4).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{
    HandlerResult, error_to_response, ok_json, ok_page, request_audit_ctx,
};
use crate::contact::{DEFAULT_STALE_DAYS, ProfileInput};

/// `GET /api/patients/{id}/communication-profile`: how to reach this patient well. Includes a
/// `fhir` Patient fragment for systems that speak FHIR.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/communication-profile",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::contact::Profile>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_profile(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match state.contact.get(id).await {
        Ok(Some(p)) => ok_json(p),
        Ok(None) => error_to_response(Error::not_found(format!(
            "no communication profile for patient {id}"
        ))),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/patients/{id}/communication-profile`: create or replace. Changing the mobile number
/// clears its verification.
#[utoipa::path(
    put,
    path = "/api/patients/{id}/communication-profile",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = ProfileInput,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::contact::Profile>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn put_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<ProfileInput>,
) -> HandlerResult {
    match state
        .contact
        .put(id, input, &request_audit_ctx(&headers))
        .await
    {
        Ok(p) => ok_json(p),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct VerifyBody {
    /// `staff`, `portal` or `sms_code`.
    pub method: String,
}

/// `POST /api/patients/{id}/communication-profile/verify-mobile`: record that the number was checked.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/communication-profile/verify-mobile",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = VerifyBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::contact::Profile>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn verify_mobile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<VerifyBody>,
) -> HandlerResult {
    match state
        .contact
        .verify_mobile(id, &body.method, &request_audit_ctx(&headers))
        .await
    {
        Ok(p) => ok_json(p),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct StaleQuery {
    pub older_than_days: Option<i64>,
    pub limit: Option<i64>,
}

/// `GET /api/communication-profiles/stale?older_than_days=365&limit=100`: profiles whose mobile is
/// missing, unverified or old, oldest first.
#[utoipa::path(
    get,
    path = "/api/communication-profiles/stale",
    tag = "Communication profiles",
    params(StaleQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::contact::Profile>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn stale_profiles(
    State(state): State<AppState>,
    Query(q): Query<StaleQuery>,
) -> HandlerResult {
    let limit = match crate::paging::limit(q.limit, 100) {
        Ok(l) => l,
        Err(e) => return error_to_response(e),
    };
    match state
        .contact
        .stale(
            q.older_than_days.unwrap_or(DEFAULT_STALE_DAYS),
            crate::paging::ask_for(limit),
        )
        .await
    {
        Ok(rows) => ok_page(rows, limit),
        Err(e) => error_to_response(e),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct OptOutBody {
    /// `sms`, `whatsapp`, `apple_messages` or `email`.
    pub channel: String,
}

/// One channel a patient has asked us not to use.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OptOut {
    pub channel: String,
    #[schema(required = true)]
    pub opted_out_at: Option<chrono::DateTime<chrono::Utc>>,
    pub source: String,
}

/// Whether a patient is opted out of a channel, after a change.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OptOutState {
    pub channel: String,
    pub opted_out: bool,
}

/// What `POST /api/validate/nhs-number` sends for a well-formed number.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NhsNumberChecked {
    pub valid: bool,
    pub digits: String,
    pub formatted: String,
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
#[utoipa::path(
    get,
    path = "/api/patients/{id}/channel-optouts",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<OptOut>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_optouts(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    let rows = state
        .db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT channel, opted_out_at, source FROM message_optouts WHERE subject = $1 AND resumed_at IS NULL ORDER BY channel",
            [crate::communication::consent::person_subject(id).into()],
        ))
        .await;
    match rows {
        Ok(rows) => {
            let list: Vec<OptOut> = rows
                .iter()
                .map(|r| OptOut {
                    channel: r.try_get::<String>("", "channel").unwrap_or_default(),
                    opted_out_at: r
                        .try_get::<chrono::DateTime<chrono::Utc>>("", "opted_out_at")
                        .ok(),
                    source: r.try_get::<String>("", "source").unwrap_or_default(),
                })
                .collect();
            ok_json(list)
        }
        Err(e) => error_to_response(Error::Database(e)),
    }
}

/// `POST /api/patients/{id}/channel-optouts`: staff record that the patient asked us to stop using a
/// channel (said on the phone, written, or through the portal). Takes effect at once.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/channel-optouts",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = OptOutBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<OptOutState>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn add_optout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<OptOutBody>,
) -> HandlerResult {
    if let Err(e) = check_channel(&body.channel) {
        return error_to_response(e);
    }
    let ctx = request_audit_ctx(&headers);
    match crate::communication::consent::record_optout(
        &state.db,
        &crate::communication::consent::person_subject(id),
        &body.channel,
        "staff",
        ctx.user_id.as_deref(),
    )
    .await
    {
        Ok(()) => ok_json(OptOutState {
            channel: body.channel,
            opted_out: true,
        }),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/patients/{id}/channel-optouts/{channel}`: the patient asked to hear from us again.
#[utoipa::path(
    delete,
    path = "/api/patients/{id}/channel-optouts/{channel}",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`"), ("channel" = String, Path, description = "Path parameter `channel`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<OptOutState>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
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
        Ok(true) => ok_json(OptOutState {
            channel,
            opted_out: false,
        }),
        Ok(false) => error_to_response(Error::not_found(
            "that patient has not opted out of that channel",
        )),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct NhsNumberBody {
    pub value: String,
}

/// `POST /api/validate/nhs-number` (DTAC-8): check an NHS number's format and modulus-11 check digit. It says
/// whether the number is well formed, not whether it is issued or belongs to anyone (that needs a PDS trace,
/// see `governance/pds-decision.md`). Stores nothing.
#[utoipa::path(
    post,
    path = "/api/validate/nhs-number",
    tag = "Validate",
    request_body = NhsNumberBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<NhsNumberChecked>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn validate_nhs_number(Json(b): Json<NhsNumberBody>) -> HandlerResult {
    match crate::validation::validate_nhs_number(&b.value) {
        Ok(n) => ok_json(NhsNumberChecked {
            valid: true,
            formatted: crate::validation::format_nhs_number(&n),
            digits: n,
        }),
        Err(e) => error_to_response(e),
    }
}
