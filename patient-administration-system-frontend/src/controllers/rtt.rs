//! RTT cockpit controller — one page at `/rtt`.

use crate::tpl::json;
use crate::views::Views;
use axa::prelude::*;
use axum::Extension;

use crate::models::rtt::list_active_pathways;

pub fn routes() -> Routes {
    Routes::new().prefix("/rtt").add("/", get(index))
}

/// `GET /rtt` — every non-stopped RTT pathway with computed weeks-waiting,
/// sorted worst-breach-first.
pub async fn index(
    Extension(v): Extension<Views>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let pathways = list_active_pathways(&ctx.db).await.map_err(Error::wrap)?;
    let breach_count = pathways.iter().filter(|p| p.is_breaching).count();
    v.render(
        "rtt/index.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "pathways": pathways,
            "total_pathways": pathways.len(),
            "breach_count": breach_count,
        }),
    )
}
