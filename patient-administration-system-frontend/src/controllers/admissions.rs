//! Admission flow — `GET /admissions/new` form + `POST /admissions/new`
//! submit. The actual admission writes go through the PAS Axum API so
//! the transactional `AdtService::admit` path (encounter creation, bed
//! lock + flip to occupied, audit + outbox) runs in the system of record.

use crate::tpl::json;
use crate::views::Views;
use axa::prelude::*;
use axum::Extension;
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::csrf;
use crate::models::{bed::list_available, patient::list_active_patients};
use crate::services::pas_api::{AdmitRequest, PasApiError, admit};

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/admissions")
        .add("/new", get(new_form).post(submit))
}

/// `GET /admissions/new` — render the form. Sets the `pas_csrf` cookie
/// (if missing) and threads the matching token into the hidden form
/// field so the subsequent POST can pass [`csrf::verify_token`].
pub async fn new_form(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
    jar: CookieJar,
) -> Result<(CookieJar, Response)> {
    let (csrf_token, jar) = csrf::ensure_token(jar, &ctx.environment);
    let patients = list_active_patients(200).await;
    let beds = list_available(&ctx.db).await.map_err(Error::wrap)?;
    let resp = v.render(
        "admissions/new.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "patients": patients,
            "beds": beds,
            "csrf_token": csrf_token,
            "error": serde_json::Value::Null,
            "result": serde_json::Value::Null,
        }),
    )?;
    Ok((jar, resp))
}

#[derive(Debug, Deserialize)]
pub struct AdmitForm {
    pub person_id: Uuid,
    pub bed_id: Uuid,
    pub csrf_token: String,
}

/// `POST /admissions/new` — verify CSRF, call `POST /api/admissions` on
/// the PAS binary, then re-render the page with the result.
pub async fn submit(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
    jar: CookieJar,
    Form(form): Form<AdmitForm>,
) -> Result<(CookieJar, Response)> {
    csrf::verify_token(&jar, &form.csrf_token)?;
    let (csrf_token, jar) = csrf::ensure_token(jar, &ctx.environment);
    let req = AdmitRequest {
        person_id: form.person_id,
        bed_id: form.bed_id,
    };
    let (error, result) = match admit(&req).await {
        Ok(out) => (
            serde_json::Value::Null,
            serde_json::json!({
                "admission_id": out.admission.id,
                "encounter_id": out.encounter.id,
                "person_id": out.encounter.person_id,
                "bed_id": out.admission.bed_id,
            }),
        ),
        Err(e) => (
            serde_json::Value::String(format_err(e)),
            serde_json::Value::Null,
        ),
    };
    let patients = list_active_patients(200).await;
    let beds = list_available(&ctx.db).await.map_err(Error::wrap)?;
    let resp = v.render(
        "admissions/new.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "patients": patients,
            "beds": beds,
            "csrf_token": csrf_token,
            "error": error,
            "result": result,
        }),
    )?;
    Ok((jar, resp))
}

fn format_err(e: PasApiError) -> String {
    match e {
        PasApiError::Http(err) => format!("PAS API HTTP: {err}"),
        PasApiError::Status { status, message } => format!("PAS API returned {status}: {message}"),
        PasApiError::MalformedResponse => "PAS API: malformed response".to_string(),
    }
}
