//! Installs the Tera view engine. Loco does not add it by itself; without this
//! layer every page that renders a template fails with "TeraLayer missing".
//! Templates are read from `assets/views/**/*.html` (relative to the working
//! directory, which is the crate root under `cargo run` and `cargo test`).

use async_trait::async_trait;
use axum::{Extension, Router as AxumRouter};
use loco_rs::{
    app::Initializer,
    controller::views::{ViewEngine, engines},
    prelude::*,
};

pub struct ViewEngineInitializer;

#[async_trait]
impl Initializer for ViewEngineInitializer {
    fn name(&self) -> String {
        "view-engine".to_string()
    }

    async fn after_routes(&self, router: AxumRouter, _ctx: &AppContext) -> Result<AxumRouter> {
        let tera = engines::TeraView::build()?;
        Ok(router.layer(Extension(ViewEngine::from(tera))))
    }
}
