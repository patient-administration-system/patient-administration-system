//! REST handlers for letter templates + generated letters.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use sea_orm::TransactionTrait;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::handlers::{audit_ctx, ok, ok_json};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::communication::templates;
use crate::db::repositories::audit::AuditLogRepository;
use crate::db::repositories::letter::LetterRepository;
use crate::models::communication::{DeliveryChannel, LetterTemplate};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct GenerateLetterBody {
    pub template_id: Uuid,
    pub person_id: Uuid,
    pub channel: DeliveryChannel,
    #[serde(default)]
    pub appointment_id: Option<Uuid>,
    #[serde(default)]
    pub extra: Value,
}

/// `POST /api/letter-templates` — create a template.
#[utoipa::path(
    post,
    path = "/api/letter-templates",
    tag = "Letter templates",
    request_body = LetterTemplate,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<LetterTemplate>,
) -> HandlerResult {
    if let Err(e) = crate::communication::validate_template_syntax(&body.subject, &body.body_tera) {
        return error_to_response(e);
    }
    if let Err(e) = templates::validate_locale(&body.locale) {
        return error_to_response(e);
    }
    if body.active {
        // An active template must come with every required language (CAP-9).
        let locales =
            match templates::active_locales_after(&state.db, &body.name, Some(&body)).await {
                Ok(l) => l,
                Err(e) => return error_to_response(e),
            };
        if let Err(e) =
            templates::check_complete(&body.name, &locales, &templates::required_locales())
        {
            return error_to_response(e);
        }
    }
    if body.id == Uuid::nil() {
        body.id = Uuid::new_v4();
    }
    body.created_at = chrono::Utc::now();
    body.updated_at = body.created_at;
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let t = LetterRepository::create_template(&txn, &body).await?;
        AuditLogRepository::log(
            &txn,
            "letter_template",
            t.id,
            "create",
            None,
            Some(serde_json::to_value(&t).unwrap_or_default()),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(t)
    };
    match run.await {
        Ok(t) => ok_json(t),
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/letter-templates/{id}` — administrators edit a template's
/// subject, body, channels, required variables and active flag. The Tera
/// source is validated before it is stored so a typo cannot break reminders.
#[utoipa::path(
    put,
    path = "/api/letter-templates/{id}",
    tag = "Letter templates",
    params(("id" = String, Path, description = "Path parameter `id`")),
    request_body = LetterTemplate,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::communication::LetterTemplate>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn update_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(mut body): Json<LetterTemplate>,
) -> HandlerResult {
    body.id = id;
    if let Err(e) = crate::communication::validate_template_syntax(&body.subject, &body.body_tera) {
        return error_to_response(e);
    }
    if let Err(e) = templates::validate_locale(&body.locale) {
        return error_to_response(e);
    }
    // After this edit the group must still be empty of active variants or have every required language.
    match templates::active_locales_after(&state.db, &body.name, Some(&body)).await {
        Ok(locales) => {
            if let Err(e) =
                templates::check_complete(&body.name, &locales, &templates::required_locales())
            {
                return error_to_response(e);
            }
        }
        Err(e) => return error_to_response(e),
    }
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let before = LetterRepository::find_template_by_id(&state.db, id).await;
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let t = LetterRepository::update_template(&txn, &body).await?;
        AuditLogRepository::log(
            &txn,
            "letter_template",
            t.id,
            "update",
            before
                .ok()
                .flatten()
                .and_then(|b| serde_json::to_value(b).ok()),
            Some(serde_json::to_value(&t).unwrap_or_default()),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(t)
    };
    match run.await {
        Ok(t) => ok_json(t),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/letter-templates/:id`.
#[utoipa::path(
    get,
    path = "/api/letter-templates/{id}",
    tag = "Letter templates",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<crate::models::communication::LetterTemplate>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_template(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match LetterRepository::find_template_by_id(&state.db, id).await {
        Ok(Some(t)) => ok_json(t),
        Ok(None) => error_to_response(Error::not_found(format!("letter template {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/letter-templates` — list active templates.
#[utoipa::path(
    get,
    path = "/api/letter-templates",
    tag = "Letter templates",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::communication::LetterTemplate>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_templates(State(state): State<AppState>) -> HandlerResult {
    match LetterRepository::list_active_templates(&state.db).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct PatientLettersQuery {
    /// `portal`, `email`, ... — omit for every channel.
    pub channel: Option<String>,
}

/// `GET /api/patients/{id}/letters?channel=portal` — a patient's letters,
/// newest first. The patient portal reads `channel=portal`.
#[utoipa::path(
    get,
    path = "/api/patients/{id}/letters",
    tag = "Patients",
    params(("id" = String, Path, description = "Path parameter `id`"), PatientLettersQuery),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope", body = crate::api::ApiResponse<Vec<crate::models::communication::GeneratedLetter>>),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn list_patient_letters(
    State(state): State<AppState>,
    Path(person_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<PatientLettersQuery>,
) -> HandlerResult {
    let channel = match q.channel.as_deref() {
        None | Some("") => None,
        Some(c) => match crate::db::repositories::letter::delivery_channel_from_str(c) {
            Ok(c) => Some(c),
            Err(_) => {
                return error_to_response(Error::validation(format!("unknown channel: {c}")));
            }
        },
    };
    match LetterRepository::list_for_person(&state.db, person_id, channel).await {
        Ok(rows) => ok_json(rows),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PreviewLetterBody {
    pub template_id: Uuid,
    pub person_id: Uuid,
    #[serde(default)]
    pub extra: Value,
}

/// `POST /api/letters/preview` — render a letter as it would be sent, without storing or sending it. A
/// missing template variable is a 400 that names it.
#[utoipa::path(
    post,
    path = "/api/letters/preview",
    tag = "Letters",
    request_body = PreviewLetterBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn preview_letter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PreviewLetterBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .communication
        .preview_letter(body.template_id, body.person_id, body.extra, &ctx)
        .await
    {
        Ok((subject, body, locale)) => ok(serde_json::json!({
            "rendered_subject": subject, "rendered_body": body, "locale": locale,
            "sms": crate::communication::sms_length::analyze(&body),
        })),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/letters/generate` — render + persist a letter (proxies to
/// `person-service` to fetch the person snapshot).
#[utoipa::path(
    post,
    path = "/api/letters/generate",
    tag = "Letters",
    request_body = GenerateLetterBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn generate_letter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GenerateLetterBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .communication
        .generate_letter(
            body.template_id,
            body.person_id,
            body.appointment_id,
            body.channel,
            body.extra,
            &ctx,
        )
        .await
    {
        Ok(l) => ok_json(l),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/letters/:id`.
#[utoipa::path(
    get,
    path = "/api/letters/{id}",
    tag = "Letters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn get_letter(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match LetterRepository::find_generated_letter_by_id(&state.db, id).await {
        Ok(Some(l)) => ok_json(l),
        Ok(None) => error_to_response(Error::not_found(format!("generated letter {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/letters/:id/sent` — operator marks a letter as delivered.
#[utoipa::path(
    post,
    path = "/api/letters/{id}/sent",
    tag = "Letters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn mark_sent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let l = LetterRepository::mark_sent(&txn, id).await?;
        AuditLogRepository::log(
            &txn,
            "generated_letter",
            id,
            "mark_sent",
            None,
            Some(serde_json::to_value(&l).unwrap_or_default()),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(l)
    };
    match run.await {
        Ok(l) => ok_json(l),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/letters/:id/failed`.
#[utoipa::path(
    post,
    path = "/api/letters/{id}/failed",
    tag = "Letters",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn mark_failed(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let run = async {
        let txn = state.db.begin().await.map_err(Error::Database)?;
        let l = LetterRepository::mark_failed(&txn, id).await?;
        AuditLogRepository::log(
            &txn,
            "generated_letter",
            id,
            "mark_failed",
            None,
            Some(serde_json::to_value(&l).unwrap_or_default()),
            &ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok::<_, Error>(l)
    };
    match run.await {
        Ok(l) => ok_json(l),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct VariantBody {
    pub locale: String,
    pub subject: String,
    pub body_tera: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct GroupBody {
    pub name: String,
    #[serde(default)]
    pub channels: Vec<DeliveryChannel>,
    #[serde(default)]
    pub required_variables: Vec<String>,
    pub variants: Vec<VariantBody>,
}

/// `POST /api/letter-template-groups`: create every language variant of one message together, or none.
/// The set must cover every required language (`PAS_REQUIRED_LOCALES`, default English and Welsh). The
/// answer shows, for each variant, how many text segments it costs: Welsh circumflex letters make a
/// text cost more than twice as much, and the writer should see that before it goes out.
#[utoipa::path(
    post,
    path = "/api/letter-template-groups",
    tag = "Letter template groups",
    request_body = GroupBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn create_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GroupBody>,
) -> HandlerResult {
    use sea_orm::TransactionTrait;
    let name = body.name.trim().to_string();
    if name.is_empty() || name.len() > 100 {
        return error_to_response(Error::validation("name must be 1 to 100 characters"));
    }
    let required = templates::required_locales();
    let mut seen: Vec<String> = Vec::new();
    for v in &body.variants {
        if let Err(e) = templates::validate_locale(&v.locale)
            .and_then(|_| crate::communication::validate_template_syntax(&v.subject, &v.body_tera))
        {
            return error_to_response(e);
        }
        if seen.contains(&v.locale) {
            return error_to_response(Error::validation(format!(
                "locale {} is listed twice",
                v.locale
            )));
        }
        seen.push(v.locale.clone());
    }
    if let Err(e) = templates::check_complete(&name, &seen, &required) {
        // An empty set is "complete" for an inactive group but is no group at all here.
        return error_to_response(if seen.is_empty() {
            Error::validation("variants must not be empty")
        } else {
            e
        });
    }
    match LetterRepository::list_active_by_name(&state.db, &name).await {
        Ok(existing) if !existing.is_empty() => {
            return error_to_response(Error::conflict(format!(
                "an active template named {name:?} already exists; edit its variants instead"
            )));
        }
        Ok(_) => {}
        Err(e) => return error_to_response(e),
    }
    let now = chrono::Utc::now();
    let channels = if body.channels.is_empty() {
        vec![DeliveryChannel::Print]
    } else {
        body.channels.clone()
    };
    let made: Vec<LetterTemplate> = body
        .variants
        .iter()
        .map(|v| {
            let mut t = LetterTemplate::new(name.clone(), v.subject.clone(), v.body_tera.clone());
            t.locale = v.locale.clone();
            t.channels = channels.clone();
            t.required_variables = body.required_variables.clone();
            t.created_at = now;
            t.updated_at = now;
            t
        })
        .collect();
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let to_save = made.clone();
    let saved = state
        .db
        .transaction::<_, Vec<LetterTemplate>, Error>(|txn| {
            Box::pin(async move {
                let mut out = Vec::new();
                for t in &to_save {
                    let s = LetterRepository::create_template(txn, t).await?;
                    AuditLogRepository::log(
                        txn,
                        "letter_template",
                        s.id,
                        "create",
                        None,
                        Some(serde_json::to_value(&s).unwrap_or_default()),
                        &ctx,
                    )
                    .await?;
                    out.push(s);
                }
                Ok(out)
            })
        })
        .await;
    match saved {
        Ok(rows) => {
            let variants: Vec<Value> = rows
                .iter()
                .map(|t| serde_json::json!({ "template": t, "sms": templates::sample_sms(&t.body_tera, &t.locale) }))
                .collect();
            ok(serde_json::json!({ "name": name, "variants": variants }))
        }
        Err(e) => error_to_response(Error::from(e)),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CheckSmsBody {
    pub body_tera: String,
    #[serde(default)]
    pub locale: Option<String>,
}

/// `POST /api/letter-templates/check-sms`: what a body would cost as a text, rendered with typical
/// values, with the character that forced the longer encoding named.
#[utoipa::path(
    post,
    path = "/api/letter-templates/check-sms",
    tag = "Letter templates",
    request_body = CheckSmsBody,
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn check_sms(Json(b): Json<CheckSmsBody>) -> HandlerResult {
    if let Err(e) = crate::communication::validate_template_syntax("", &b.body_tera) {
        return error_to_response(e);
    }
    match templates::sample_sms(&b.body_tera, b.locale.as_deref().unwrap_or("en-001")) {
        Some(len) => ok_json(len),
        None => error_to_response(Error::validation(
            "the body does not render with typical values, so its length cannot be estimated",
        )),
    }
}
