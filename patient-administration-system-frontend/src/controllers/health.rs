//! What is running, and axa's three probes.

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

/// axa's probes: `GET /_health/live` (the process is up), `GET /_health/ready` (`503` while the database cannot be
/// reached, or once shutdown begins) and `GET /_health/startup` (the database has answered once), so an orchestrator
/// stops sending traffic to an instance that cannot serve it (AXA-4). [`index`] is not a probe: it says what is
/// running.
pub fn probe_routes() -> Routes {
    axa::health::routes()
}
