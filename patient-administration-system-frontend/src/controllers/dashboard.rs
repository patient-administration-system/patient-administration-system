//! Dashboard controller — ports the PAS Axum dashboard handler into this crate's
//! controller/view shape.
//!
//! The full page lives at `/dashboard`; each panel has its own
//! HTMX-targeted fragment endpoint so the dashboard refreshes per-panel
//! without a full page swap.
//!
//! A panel that cannot read the database says so (`*_unreachable`), and logs a warning, rather than showing
//! a zero or an empty table: "0 unpublished" and "no wards" read as all clear while the database is down.

use crate::tpl::json;
use crate::views::Views;
use axa::prelude::*;
use axum::Extension;
use serde::Serialize;

use crate::models::{audit::list_recent_audit, outbox::count_unpublished, ward::list_active_wards};

const REFRESH_SECONDS: u32 = 10;

#[derive(Serialize)]
struct WardRow {
    id: uuid::Uuid,
    name: String,
    code: String,
    total_beds: usize,
    occupied: usize,
    available: usize,
    other: usize,
}

#[derive(Serialize)]
struct AuditRow {
    at_short: String,
    entity_type: String,
    action: String,
    user_id: Option<String>,
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/dashboard")
        .add("/", get(index))
        .add("/wards", get(wards_fragment))
        .add("/outbox", get(outbox_fragment))
        .add("/audit", get(audit_fragment))
}

/// `GET /dashboard` — render the full Tera page with each panel inlined,
/// so the dashboard works without JavaScript. HTMX layers on as
/// progressive enhancement.
pub async fn index(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    // At once, so that a database that does not answer costs one connect timeout, not three.
    let (wards, unpublished, audit) =
        tokio::join!(load_wards(&ctx), load_unpublished(&ctx), load_audit(&ctx));

    v.render(
        "dashboard/index.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "rendered_at": chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "refresh_seconds": REFRESH_SECONDS,
            "wards_unreachable": wards.is_none(),
            "wards": wards.unwrap_or_default(),
            "outbox_unreachable": unpublished.is_none(),
            "unpublished_count": unpublished.unwrap_or(0),
            "audit_unreachable": audit.is_none(),
            "audit": audit.unwrap_or_default(),
        }),
    )
}

/// `GET /dashboard/wards` — HTMX fragment.
pub async fn wards_fragment(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let wards = load_wards(&ctx).await;
    v.render(
        "dashboard/_wards.html",
        json!({ "wards_unreachable": wards.is_none(), "wards": wards.unwrap_or_default() }),
    )
}

/// `GET /dashboard/outbox` — HTMX fragment: single big-stat unpublished count.
pub async fn outbox_fragment(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let unpublished = load_unpublished(&ctx).await;
    v.render(
        "dashboard/_outbox.html",
        json!({
            "outbox_unreachable": unpublished.is_none(),
            "unpublished_count": unpublished.unwrap_or(0),
        }),
    )
}

/// `GET /dashboard/audit` — HTMX fragment: last 10 audit rows.
pub async fn audit_fragment(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let audit = load_audit(&ctx).await;
    v.render(
        "dashboard/_audit.html",
        json!({ "audit_unreachable": audit.is_none(), "audit": audit.unwrap_or_default() }),
    )
}

// ---- data loaders ---------------------------------------------------------

/// `None`, and a warning, when the database cannot be read.
fn unreachable<T, E: std::fmt::Display>(
    panel: &str,
    result: std::result::Result<T, E>,
) -> Option<T> {
    result
        .inspect_err(
            |e| tracing::warn!(panel, error = %e, "a dashboard panel could not read the database"),
        )
        .ok()
}

async fn load_unpublished(ctx: &AppContext) -> Option<usize> {
    unreachable("outbox", count_unpublished(&ctx.db).await)
}

async fn load_wards(ctx: &AppContext) -> Option<Vec<WardRow>> {
    let mut rows = Vec::new();
    let wards = unreachable("wards", list_active_wards(&ctx.db).await)?;
    for w in wards {
        let beds = unreachable(
            "wards",
            crate::models::bed::list_by_ward(&ctx.db, w.id).await,
        )?;
        let total_beds = beds.len();
        let mut occupied = 0;
        let mut available = 0;
        let mut other = 0;
        for b in &beds {
            match b.status.as_str() {
                "occupied" => occupied += 1,
                "available" => available += 1,
                _ => other += 1,
            }
        }
        rows.push(WardRow {
            id: w.id,
            name: w.name,
            code: w.code,
            total_beds,
            occupied,
            available,
            other,
        });
    }
    rows.sort_by(|a, b| a.code.cmp(&b.code));
    Some(rows)
}

async fn load_audit(ctx: &AppContext) -> Option<Vec<AuditRow>> {
    let rows = unreachable("audit", list_recent_audit(&ctx.db, 10).await)?;
    let rows = rows
        .into_iter()
        .map(|m| AuditRow {
            at_short: m.at.format("%Y-%m-%d %H:%M:%S").to_string(),
            entity_type: m.entity_type,
            action: m.action,
            user_id: m.user_id,
        })
        .collect();
    Some(rows)
}
