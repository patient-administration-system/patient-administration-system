//! Upstream HTTP clients for `person-service` and `worker-service`.
//!
//! Phase 1 of the v0.44.0 Option C identity delegation. Adds **additive**
//! HTTP client traits + reqwest impls + in-memory stub impls for tests.
//! No existing handler calls into this module yet — Phase 3 rewrites the
//! Patient and Practitioner handlers to proxy through these clients.
//!
//! Design rules baked in here (per `spec.md` §13 Layer 11A):
//!
//! - **HTTP-only.** The PAS does not depend on the upstream crates at compile
//!   time. The wire shape (whatever JSON the upstream returns) is the
//!   contract. Bodies and responses are kept loosely typed (`serde_json::Map`
//!   flattened into [`Person`] / [`Worker`]) so the upstream can evolve
//!   without breaking PAS compile.
//! - **User pass-through.** Every call accepts a [`UserContext`] and attaches
//!   the inbound `X-User-Id` / `X-User-Ip` / `X-User-Agent` headers to the
//!   outbound request so upstream audit credits the original caller.
//! - **Bearer auth.** When configured with `PERSON_SERVICE_TOKEN` /
//!   `WORKER_SERVICE_TOKEN`, the HTTP impls attach
//!   `Authorization: Bearer <token>` on every request.
//! - **Typed error mapping.** Translatable upstream 4xx envelopes
//!   (`{success: false, error: {code, message}}`) become matching
//!   [`crate::Error`] variants. Anything else (transport, TLS, DNS, read
//!   error, non-translatable non-2xx) is [`crate::Error::Upstream`].
//! - **Stubs for tests.** [`StubPersonClient`] and
//!   [`StubWorkerClient`] are pure in-memory `HashMap` implementations
//!   that satisfy the trait without any HTTP at all. Integration tests in
//!   later phases can install them via `AppState`.

pub mod person_client;
pub mod worker_client;

pub use person_client::{
    HttpPersonClient, Person, PersonClient, PersonCreate, PersonMatch, StubPersonClient,
};
pub use worker_client::{
    HttpWorkerClient, StubWorkerClient, Worker, WorkerClient, WorkerCreate, WorkerMatch,
};

use uuid::Uuid;

/// Caller identity to thread through to the upstream service so its audit
/// log credits the original PAS-side caller (not the PAS service account).
///
/// Built from inbound `X-User-Id` / `X-User-Ip` / `X-User-Agent` headers in
/// the handler. All three are optional; `Default` produces a context with
/// every field `None`.
#[derive(Debug, Clone, Default)]
pub struct UserContext {
    pub user_id: Option<String>,
    pub user_ip: Option<String>,
    pub user_agent: Option<String>,
}

impl UserContext {
    /// Anonymous caller — used for system-level paths (outbox dispatcher,
    /// webhook receivers, startup health probe) that have no inbound user.
    pub fn system() -> Self {
        Self::default()
    }
}

/// One audit-log entry as returned by an upstream `/api/{persons,workers}/{id}/audit`
/// query. Kept deliberately loose — the upstream's exact shape may evolve.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct UpstreamAuditEntry {
    pub id: Uuid,
    pub at: chrono::DateTime<chrono::Utc>,
    pub action: String,
    pub user_id: Option<String>,
    #[serde(default)]
    pub details: serde_json::Value,
}

/// Outcome of an upstream merge call.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct MergeOutcome {
    pub source_id: Uuid,
    pub target_id: Uuid,
}
