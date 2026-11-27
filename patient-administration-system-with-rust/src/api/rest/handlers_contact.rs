//! REST handlers for contact reliability and communication needs (CAP-4).

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Stmt};
use axa::extract::{Json, Path, Query};
use axum::{extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response, ok_json, request_audit_ctx};
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
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
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
}

/// `GET /api/communication-profiles/stale?older_than_days=365&limit=100`: profiles whose mobile is
/// missing, unverified or old, oldest first.
#[utoipa::path(
    get,
    path = "/api/communication-profiles/stale",
    tag = "Communication profiles",
    params(crate::paging::PageQuery, StaleQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::contact::Profile>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn stale_profiles(
    State(state): State<AppState>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    Query(q): Query<StaleQuery>,
) -> HandlerResult {
    let page = match crate::paging::Page::new(&uri, 100) {
        Ok(p) => p,
        Err(e) => return error_to_response(e),
    };
    match state
        .contact
        .stale(q.older_than_days.unwrap_or(DEFAULT_STALE_DAYS), page.rows())
        .await
    {
        Ok(rows) => crate::api::rest::handlers::ok_cursor_page(&page, rows),
        Err(e) => error_to_response(e),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct OptOutBody {
    /// `sms`, `whatsapp`, `apple_messages` or `email`.
    #[schema(value_type = crate::models::communication::PatientOptOutChannel)]
    pub channel: String,
}

/// One channel a patient has asked us not to use.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OptOut {
    pub channel: crate::models::communication::OptOutChannel,
    #[schema(required = true)]
    pub opted_out_at: Option<jiff::Timestamp>,
    pub source: crate::models::communication::OptOutSource,
}

/// Whether a patient is opted out of a channel, after a change.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OptOutState {
    pub channel: crate::models::communication::PatientOptOutChannel,
    pub opted_out: bool,
}

/// `GET /api/patients/{id}/channel-optouts`: the channels this patient has asked us not to use.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/channel-optouts",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<OptOut>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn list_optouts(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    let rows = state
        .db
        .all(Stmt::new("SELECT channel, opted_out_at, source FROM message_optouts WHERE subject = $1 AND resumed_at IS NULL ORDER BY channel",
            [crate::communication::consent::person_subject(id).into()],
        ))
        .await;
    match rows {
        Ok(rows) => {
            let list: Result<Vec<OptOut>, Error> = rows
                .iter()
                .map(|r| {
                    Ok(OptOut {
                        channel: r.col::<String>("channel")?.parse().map_err(|_| {
                            Error::internal("message_optouts has an unknown channel")
                        })?,
                        opted_out_at: r.col::<Option<jiff::Timestamp>>("opted_out_at")?,
                        source: r.col::<String>("source")?.parse().map_err(|_| {
                            Error::internal("message_optouts has an unknown source")
                        })?,
                    })
                })
                .collect();
            match list {
                Ok(list) => ok_json(list),
                Err(e) => error_to_response(e),
            }
        }
        Err(e) => error_to_response(e),
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
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn add_optout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<OptOutBody>,
) -> HandlerResult {
    let channel = match body
        .channel
        .parse::<crate::models::communication::PatientOptOutChannel>()
    {
        Ok(channel) => channel,
        Err(e) => return error_to_response(e),
    };
    let ctx = request_audit_ctx(&headers);
    match crate::communication::consent::record_optout(
        &state.db,
        &crate::communication::consent::person_subject(id),
        channel.into(),
        crate::models::communication::OptOutSource::Staff,
        ctx.user_id.as_deref(),
    )
    .await
    {
        Ok(()) => ok_json(OptOutState {
            channel,
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
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn remove_optout(
    State(state): State<AppState>,
    Path((id, channel)): Path<(Uuid, String)>,
) -> HandlerResult {
    let channel = match channel.parse::<crate::models::communication::PatientOptOutChannel>() {
        Ok(channel) => channel,
        Err(_) => return error_to_response(Error::validation("unknown opt-out channel")),
    };
    match crate::communication::consent::resume(
        &state.db,
        &crate::communication::consent::person_subject(id),
        channel.into(),
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

#[derive(Deserialize, utoipa::ToSchema)]
pub struct KindRefusalBody {
    /// `survey` or `experiment`.
    #[schema(value_type = crate::models::communication::OptionalKind)]
    pub kind: String,
}

/// One kind of message a patient has refused (TSC-11).
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KindRefusal {
    pub kind: crate::models::communication::OptionalKind,
    pub refused_at: jiff::Timestamp,
    pub source: crate::models::communication::OptOutSource,
}

/// Whether a patient refuses a kind of message, after a change.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KindRefusalState {
    pub kind: crate::models::communication::OptionalKind,
    pub refused: bool,
}

/// `GET /api/patients/{id}/message-kind-refusals`: the kinds of message (surveys, wording experiments) this
/// patient has refused. Appointment messages are never among them; the channel opt-outs govern those.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/message-kind-refusals",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<KindRefusal>>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn list_kind_refusals(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> HandlerResult {
    let rows = state
        .db
        .all(Stmt::new(
            "SELECT kind, refused_at, source FROM message_kind_refusals WHERE subject = $1 AND resumed_at IS NULL ORDER BY kind",
            [crate::communication::consent::person_subject(id).into()],
        ))
        .await;
    let list = rows.and_then(|rows| {
        rows.iter()
            .map(|r| {
                Ok(KindRefusal {
                    kind: r.col::<String>("kind")?.parse().map_err(|_| {
                        Error::internal("message_kind_refusals has an unknown kind")
                    })?,
                    refused_at: r.col("refused_at")?,
                    source: r.col::<String>("source")?.parse().map_err(|_| {
                        Error::internal("message_kind_refusals has an unknown source")
                    })?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()
    });
    match list {
        Ok(list) => ok_json(list),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/patients/{id}/message-kind-refusals`: staff record that the patient refuses a kind of message
/// (surveys, or being in a wording experiment). Appointment messages carry on. Takes effect at once.
#[utoipa::path(
    post,
    path = "/api/patients/{id}/message-kind-refusals",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = KindRefusalBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<KindRefusalState>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn add_kind_refusal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<KindRefusalBody>,
) -> HandlerResult {
    let kind = match body
        .kind
        .parse::<crate::models::communication::OptionalKind>()
    {
        Ok(kind) => kind,
        Err(e) => return error_to_response(e),
    };
    let ctx = request_audit_ctx(&headers);
    match crate::communication::consent::refuse_kind(
        &state.db,
        &crate::communication::consent::person_subject(id),
        kind,
        crate::models::communication::OptOutSource::Staff,
        ctx.user_id.as_deref(),
    )
    .await
    {
        Ok(()) => ok_json(KindRefusalState {
            kind,
            refused: true,
        }),
        Err(e) => error_to_response(e),
    }
}

/// `DELETE /api/patients/{id}/message-kind-refusals/{kind}`: the patient will take that kind of message again.
#[utoipa::path(
    delete,
    path = "/api/patients/{id}/message-kind-refusals/{kind}",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`"), ("kind" = String, Path, description = "Path parameter `kind`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<KindRefusalState>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn remove_kind_refusal(
    State(state): State<AppState>,
    Path((id, kind)): Path<(Uuid, String)>,
) -> HandlerResult {
    let kind = match kind.parse::<crate::models::communication::OptionalKind>() {
        Ok(kind) => kind,
        Err(_) => return error_to_response(Error::validation("unknown kind of message")),
    };
    match crate::communication::consent::resume_kind(
        &state.db,
        &crate::communication::consent::person_subject(id),
        kind,
    )
    .await
    {
        Ok(true) => ok_json(KindRefusalState {
            kind,
            refused: false,
        }),
        Ok(false) => error_to_response(Error::not_found(
            "that patient has not refused that kind of message",
        )),
        Err(e) => error_to_response(e),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct TemplateKindBody {
    /// `survey`, or none to make the group a service message again.
    #[schema(value_type = Option<crate::models::communication::TemplateKind>)]
    pub kind: Option<String>,
}

/// A template group's kind, after a change.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TemplateKindState {
    pub template_name: String,
    /// None for a service message.
    #[schema(required = true)]
    pub kind: Option<crate::models::communication::TemplateKind>,
}

/// `PUT /api/letter-templates/kinds/{name}`: say what the template group `name` (every language variant) is:
/// `survey`, which a patient can refuse on its own, or none, a service message governed only by the channel
/// opt-outs (TSC-11).
#[utoipa::path(
    put,
    path = "/api/letter-templates/kinds/{name}",
    tag = "Letters",
    params(("name" = String, Path, description = "The template group's name")),
    request_body = TemplateKindBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<TemplateKindState>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn put_template_kind(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<TemplateKindBody>,
) -> HandlerResult {
    let kind = match body
        .kind
        .as_deref()
        .map(str::parse::<crate::models::communication::TemplateKind>)
    {
        None => None,
        Some(Ok(k)) => Some(k),
        Some(Err(e)) => return error_to_response(e),
    };
    let known = state
        .db
        .one(Stmt::new(
            "SELECT 1 AS hit FROM letter_templates WHERE name = $1 LIMIT 1",
            [name.clone().into()],
        ))
        .await;
    match known {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error_to_response(Error::not_found(format!("letter template group {name}")));
        }
        Err(e) => return error_to_response(e),
    }
    let ctx = request_audit_ctx(&headers);
    let done = match kind {
        Some(k) => {
            state
                .db
                .run(Stmt::new(
                    "INSERT INTO template_message_kinds (template_name, kind, set_by) VALUES ($1,$2,$3) \
                     ON CONFLICT (template_name) DO UPDATE SET kind = EXCLUDED.kind, set_by = EXCLUDED.set_by, updated_at = now()",
                    [name.clone().into(), k.as_str().into(), ctx.user_id.clone().into()],
                ))
                .await
        }
        None => {
            state
                .db
                .run(Stmt::new(
                    "DELETE FROM template_message_kinds WHERE template_name = $1",
                    [name.clone().into()],
                ))
                .await
        }
    };
    match done {
        Ok(_) => ok_json(TemplateKindState {
            template_name: name,
            kind,
        }),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct IdentifierBody {
    /// The identifier system URI, as in FHIR `Identifier.system`.
    pub system: String,
    pub value: String,
}

/// `POST /api/validate/identifier` (DTAC-8, QUAL-40): check a value against the scheme its system names. It
/// says whether the value is well formed, not whether it was issued or belongs to anyone. A system with no
/// scheme is a `400`. Stores nothing.
#[utoipa::path(
    post,
    path = "/api/validate/identifier",
    tag = "Validate",
    request_body = IdentifierBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::identifiers::IdentifierChecked>),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn validate_identifier(Json(b): Json<IdentifierBody>) -> HandlerResult {
    match crate::identifiers::check(&b.system, &b.value) {
        Ok(c) => ok_json(c),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/identifier-schemes` (QUAL-40): the identifier schemes the PAS checks, each with its system URI.
#[utoipa::path(
    get,
    path = "/api/identifier-schemes",
    tag = "Validate",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::identifiers::IdentifierScheme>>),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn list_identifier_schemes() -> HandlerResult {
    let schemes: Vec<crate::identifiers::IdentifierScheme> =
        crate::identifiers::SCHEMES.iter().map(Into::into).collect();
    ok_json(schemes)
}
