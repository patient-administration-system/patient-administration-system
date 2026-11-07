//! Patients controller — list + detail pages.

use crate::tpl::json;
use crate::views::Views;
use axa::extract::Path;
use axa::prelude::*;
use axum::Extension;
use uuid::Uuid;

use crate::models::patient::{find_patient_by_id, list_active_patients};

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/patients")
        .add("/", get(index))
        .add("/{id}", get(show))
}

/// `GET /patients` — list active patients.
pub async fn index(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let _ = &ctx;
    let patients = list_active_patients(100).await;
    v.render(
        "patients/index.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "patients": patients,
        }),
    )
}

/// `GET /patients/:id` — patient detail.
pub async fn show(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let _ = &ctx;
    let Some(p) = find_patient_by_id(id)
        .await
        .map_err(|e| Error::Message(e.to_string()))?
    else {
        return Err(Error::NotFound);
    };
    v.render(
        "patients/show.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "patient": p,
        }),
    )
}
