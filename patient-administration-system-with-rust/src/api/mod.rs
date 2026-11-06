//! api

pub mod dashboard;
pub mod fhir;
pub mod openapi;
pub mod rest;

pub use openapi::ApiDoc;

use serde::{Deserialize, Serialize};

/// Envelope for every successful JSON response from the PAS REST API. A refusal or failure is not an envelope but
/// an RFC 9457 problem: see [`problem`].
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    /// Present on list responses: the limit that applied and whether the list was cut short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<crate::paging::PageMeta>,
}

/// The body of every refused or failed REST request: an RFC 9457 problem (`application/problem+json`, FR-206), with
/// the status's reason as `title`, `message` as `detail`, and the machine-readable `code` as an extension.
///
/// Never pass internal failure text: `detail` goes to the client ([`crate::Error::client_message`] is the safe text
/// of an error).
///
/// ```
/// use axum::http::StatusCode;
/// use patient_administration_system::api::problem;
///
/// let p = problem(StatusCode::CONFLICT, "CONFLICT", "that slot has gone");
/// assert_eq!(p.title, "Conflict");
/// assert_eq!(p.status, 409);
/// assert_eq!(p.extensions["code"], "CONFLICT");
/// ```
pub fn problem(
    status: axum::http::StatusCode,
    code: &str,
    message: impl Into<String>,
) -> axa_problem::Problem {
    axa_problem::Problem::new(status)
        .with_detail(message)
        .with_extension("code", serde_json::Value::from(code))
}

/// The `data` of a success that has nothing to return: it serializes to `null`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Nothing;

impl<T> ApiResponse<T> {
    /// Build a success response carrying `data`.
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            meta: None,
        }
    }

    /// Build a success response for a list, with the page information.
    pub fn ok_page(data: T, meta: crate::paging::PageMeta) -> Self {
        Self {
            meta: Some(meta),
            ..Self::ok(data)
        }
    }
}
