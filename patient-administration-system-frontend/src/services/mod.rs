//! Outbound service clients.
//!
//! When patient-administration-system-frontend needs to *mutate* PAS data, it calls the PAS Axum
//! REST API (so audit + outbox stay in the system of record) rather than
//! writing to the shared database directly.

pub mod pas_api;
pub mod person_api;

/// The headers that tie a call to the request it serves (AXA-6): the same `X-Request-Id`, so one
/// failing page can be followed through this service's logs and its peer's.
fn request_id() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    axa::tracing::propagate(&mut headers);
    headers
}
