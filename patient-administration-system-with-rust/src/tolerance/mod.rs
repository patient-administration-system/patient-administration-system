//! Tolerances (red / amber / green): how far a measure may vary from its target.
//!
//! Each measure has a **target**, a **tolerance** (the drift that is still fine) and an **escalate-beyond**
//! limit (the drift past which someone must act). The adverse drift is how far the value has moved in the
//! worse direction from the target:
//!
//! - **green**: adverse drift is within the tolerance.
//! - **amber**: past the tolerance, but within the escalate-beyond limit. Watch it; tell the owner.
//! - **red**: past the limit. Escalate.
//! - **no data**: nothing to measure yet. Never shown as green.
//!
//! Measures cover time (waits, lead time), money (invoices collected) and resources (slot use, bed
//! occupancy, missed appointments). The built-in defaults are starting points and a named person changes
//! them (`PUT /api/tolerances/{metric}`); every change is audited. A tolerance can also be set for one service
//! or one ward, and where none is set the whole service's applies. When a measure goes red an alert opens (one
//! per measure and scope, and a `ToleranceAlertRaised` event); it closes when the measure is no longer red. Nothing here acts on a patient: it is a
//! view for managers, and no score or rank of a person takes part (NFR-11).

use crate::db::sql::Pool;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::reports::{GroupBy, ReportService};
use crate::{Error, Result};

mod alerts;
mod settings;
mod status;
pub use status::page;
// The measures, tiers, scopes and checks are domain rules (QUAL-52).
pub use patient_administration_system_domain::tolerance::*;

pub struct ToleranceService {
    db: Pool,
    reports: ReportService,
}

/// A stored tolerance row.
struct Row {
    scope: Scope,
    metric: String,
    limits: Limits,
    set_by: Option<String>,
    note: Option<String>,
    updated_at: Option<DateTime<Utc>>,
}

impl ToleranceService {
    pub fn new(db: Pool) -> Self {
        Self {
            reports: ReportService::new(db.clone()),
            db,
        }
    }
}
