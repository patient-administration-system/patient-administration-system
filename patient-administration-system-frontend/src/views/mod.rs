//! The Tera templates in `assets/views/**/*.html`, read from the working directory, which is the
//! crate root under `cargo run` and `cargo test`.
//!
//! Tera's own settings apply: HTML is escaped unless a template says otherwise. They load once, so
//! a template that does not parse stops the boot, and a change to one needs a restart.

use std::sync::Arc;

use axa::{Error, Result, format};
use axum::response::Response;
use serde::Serialize;
use tera::{Context, Tera};

const TEMPLATES: &str = "assets/views/**/*.html";

/// The loaded templates, in the request extensions as `Extension<Views>`.
#[derive(Clone)]
pub struct Views(Arc<Tera>);

impl Views {
    /// Loads every template.
    ///
    /// # Errors
    ///
    /// Fails when a template cannot be read or parsed.
    pub fn load() -> Result<Self> {
        // All in one call: Tera 2 resolves `{% extends %}` as templates are added, so a child
        // added before its parent would be refused.
        let mut templates = Vec::new();
        for (file, name) in tera::load_from_glob(TEMPLATES).map_err(Error::wrap)? {
            templates.push((name, std::fs::read_to_string(&file)?));
        }
        let mut tera = Tera::default();
        tera.add_raw_templates(templates).map_err(Error::wrap)?;
        Ok(Self(Arc::new(tera)))
    }

    /// `template` with `data`, as an HTML response.
    ///
    /// # Errors
    ///
    /// Fails when the template does not exist or does not render.
    pub fn render<S: Serialize>(&self, template: &str, data: S) -> Result<Response> {
        let context = Context::from_serialize(&data).map_err(Error::wrap)?;
        let html = self.0.render(template, &context).map_err(Error::wrap)?;
        format::html(&html)
    }
}
