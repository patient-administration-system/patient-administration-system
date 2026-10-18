//! REST handlers for letter templates + generated letters.

use axum::{
    Json,
    extract::{Path, State},
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
use crate::db::repositories::audit::{AuditLogRepository, UserContext as RepoUserContext};
use crate::db::repositories::letter::LetterRepository;
use crate::models::communication::{DeliveryChannel, LetterTemplate};

#[derive(Debug, Deserialize)]
pub struct GenerateLetterBody {
    pub template_id: Uuid,
    pub person_id: Uuid,
    pub channel: DeliveryChannel,
    #[serde(default)]
    pub appointment_id: Option<Uuid>,
    #[serde(default)]
    pub extra: Value,
}

fn audit_ctx(uc: &crate::upstream::UserContext) -> RepoUserContext {
    RepoUserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    }
}

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `POST /api/letter-templates` — create a template.
pub async fn create_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<LetterTemplate>,
) -> HandlerResult {
    if let Err(e) = crate::communication::validate_template_syntax(&body.subject, &body.body_tera) {
        return error_to_response(e);
    }
    if body.id == Uuid::nil() {
        body.id = Uuid::new_v4();
    }
    body.created_at = chrono::Utc::now();
    body.updated_at = body.created_at;
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match LetterRepository::create_template(&state.db, &body).await {
        Ok(t) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "letter_template",
                t.id,
                "create",
                None,
                Some(serde_json::to_value(&t).unwrap_or_default()),
                &ctx,
            )
            .await;
            ok(serde_json::to_value(t).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `PUT /api/letter-templates/{id}` — administrators edit a template's
/// subject, body, channels, required variables and active flag. The Tera
/// source is validated before it is stored so a typo cannot break reminders.
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
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let before = LetterRepository::find_template_by_id(&state.db, id).await;
    match LetterRepository::update_template(&state.db, &body).await {
        Ok(t) => {
            let _ = AuditLogRepository::log(
                &state.db,
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
            .await;
            ok(serde_json::to_value(t).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/letter-templates/:id`.
pub async fn get_template(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match LetterRepository::find_template_by_id(&state.db, id).await {
        Ok(Some(t)) => ok(serde_json::to_value(t).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!("letter template {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/letter-templates` — list active templates.
pub async fn list_templates(State(state): State<AppState>) -> HandlerResult {
    match LetterRepository::list_active_templates(&state.db).await {
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PatientLettersQuery {
    /// `portal`, `email`, ... — omit for every channel.
    pub channel: Option<String>,
}

/// `GET /api/patients/{id}/letters?channel=portal` — a patient's letters,
/// newest first. The patient portal reads `channel=portal`.
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
        Ok(rows) => ok(serde_json::to_value(rows).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/letters/generate` — render + persist a letter (proxies to
/// `person-service` to fetch the person snapshot).
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
        Ok(l) => ok(serde_json::to_value(l).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `GET /api/letters/:id`.
pub async fn get_letter(State(state): State<AppState>, Path(id): Path<Uuid>) -> HandlerResult {
    match LetterRepository::find_generated_letter_by_id(&state.db, id).await {
        Ok(Some(l)) => ok(serde_json::to_value(l).unwrap_or(Value::Null)),
        Ok(None) => error_to_response(Error::not_found(format!("generated letter {id}"))),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/letters/:id/sent` — operator marks a letter as delivered.
pub async fn mark_sent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match LetterRepository::mark_sent(&state.db, id).await {
        Ok(l) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "generated_letter",
                id,
                "mark_sent",
                None,
                Some(serde_json::to_value(&l).unwrap_or_default()),
                &ctx,
            )
            .await;
            ok(serde_json::to_value(l).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/letters/:id/failed`.
pub async fn mark_failed(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match LetterRepository::mark_failed(&state.db, id).await {
        Ok(l) => {
            let _ = AuditLogRepository::log(
                &state.db,
                "generated_letter",
                id,
                "mark_failed",
                None,
                Some(serde_json::to_value(&l).unwrap_or_default()),
                &ctx,
            )
            .await;
            ok(serde_json::to_value(l).unwrap_or(Value::Null))
        }
        Err(e) => error_to_response(e),
    }
}
