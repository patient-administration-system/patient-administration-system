//! Trivial liveness probe — confirms Loco booted and can reach the DB.

use loco_rs::prelude::*;
use serde_json::Value;

pub fn routes() -> Routes {
    Routes::new().prefix("/_health").add("/", get(index))
}

pub async fn index(State(ctx): State<AppContext>) -> Result<Response> {
    let db_ok = ctx.db.ping().await.is_ok();
    let body: Value = serde_json::json!({
        "service": "patient-administration-system-frontend",
        "version": env!("CARGO_PKG_VERSION"),
        "database": if db_ok { "ok" } else { "unreachable" },
    });
    format::json(body)
}

/// `GET /_ping` — process liveness with no database call (Loco's own `/_ping`
/// is not reachable once the default routes are replaced; see `app.rs`).
pub fn ping_routes() -> Routes {
    Routes::new().add("/_ping", get(ping))
}

pub async fn ping() -> Result<Response> {
    format::json(serde_json::json!({ "ok": true }))
}
