//! The REST API's routing table: `router` merges one function per domain (people, workforce, ADT,
//! scheduling, messaging, waiting, support, reporting, records, HL7 v2), each listing its own routes
//! (QUAL-30). Every route also carries a `#[utoipa::path]` on its handler, and the OpenAPI test fails when
//! a route here is missing from the document. All methods of one path live in the same function.
//!
//! The operator dashboard (`/dashboard` and its fragments) is merged in by `main.rs`, not here.

// One function per domain, each short enough to read (QUAL-30).
#![deny(clippy::too_many_lines)]

use axum::{
    Router,
    routing::{delete, get, post, put},
};

use super::AppState;
use super::handlers;
use super::handlers_admin;
use super::handlers_adt;
use super::handlers_analytics;
use super::handlers_audit;
use super::handlers_billing;
use super::handlers_clinical_priority;
use super::handlers_consent;
use super::handlers_contact;
use super::handlers_coverage;
use super::handlers_encounter;
use super::handlers_experiments;
use super::handlers_follow_up_delay;
use super::handlers_followup;
use super::handlers_group_sessions;
use super::handlers_history;
use super::handlers_hl7v2;
use super::handlers_identity;
use super::handlers_interpreter;
use super::handlers_ivr;
use super::handlers_letters;
use super::handlers_links;
use super::handlers_needs_review;
use super::handlers_noshow;
use super::handlers_offers;
use super::handlers_print;
use super::handlers_receipts;
use super::handlers_referrals;
use super::handlers_reports;
use super::handlers_resources;
use super::handlers_scheduling;
use super::handlers_series;
use super::handlers_standby;
use super::handlers_support;
use super::handlers_tolerance;
use super::handlers_transport;
use super::handlers_validation;
use super::handlers_virtual;
use super::handlers_waitlist;
use super::handlers_waits;
use super::handlers_whatsapp;
use super::handlers_writeback;
use super::upstream_proxy;

mod clinical;
mod directory;
mod hl7;
mod messaging_waiting;
mod reports_records;
use clinical::*;
use directory::*;
use hl7::*;
use messaging_waiting::*;
use reports_records::*;

/// Build the REST sub-router and attach the shared [`AppState`].
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(people())
        .merge(workforce())
        .merge(adt())
        .merge(scheduling())
        .merge(messaging())
        .merge(waiting())
        .merge(support())
        .merge(reporting())
        .merge(records())
        .merge(hl7v2())
        // Dashboard (/dashboard + 4 fragment routes) is mounted from
        // `main.rs` as a sibling router that merges in at boot time —
        // it isn't part of this REST table.
        .with_state(state)
}
