//! Trivial liveness probe — confirms the service booted and can reach the DB.

use axa::prelude::*;
use serde_json::Value;

pub fn routes() -> Routes {
    Routes::new().prefix("/_health").add("/", get(index))
}

pub async fn index(State(ctx): State<AppContext>) -> Result<Response> {
    let db_ok = axa::sqlx::ping(&ctx.db).await.is_ok();
    let body: Value = serde_json::json!({
        "service": "patient-administration-system-frontend",
        "version": env!("CARGO_PKG_VERSION"),
        "database": if db_ok { "ok" } else { "unreachable" },
    });
    format::json(body)
}

/// `GET /_ping` (the process is up) and `GET /_readiness` (`503` while the database cannot be reached), from
/// axa, so an orchestrator stops sending traffic to an instance that cannot serve it (AXA-4). axa's own `/_health`
/// is left out: [`index`] says more.
pub fn probe_routes() -> Routes {
    let mut routes = axa::health::routes();
    routes.handlers.retain(|h| h.uri != "/_health");
    routes
}
