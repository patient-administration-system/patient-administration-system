//! adt — Admission/Discharge/Transfer service
//!
//! Orchestrates the ADT state machine on top of the persistence layer. Each
//! public method runs inside a single DB transaction so that:
//!
//! - the `Bed` row is locked with `SELECT … FOR UPDATE` before being touched,
//! - the entity row(s), audit log row, and outbox event row are all written
//!   in the same transaction (transactional outbox pattern),
//! - on commit, the service performs a best-effort publish via the configured
//!   [`EventPublisher`]. The outbox row remains the durable record of the
//!   event; the publish is a fire-and-forget convenience for in-process
//!   consumers.

use sea_orm::{DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::{
    admission::AdmissionRepository,
    audit::{AuditLogRepository, UserContext},
    bed::BedRepository,
    encounter::EncounterRepository,
    outbox::OutboxRepository,
};
use crate::error::LogErr;
use crate::models::admission::{Admission, BedAssignment, Discharge, Transfer};
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::models::facility::BedStatus;
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

mod admission;
mod discharge;
mod leave;
mod movements;

/// The combined result of a successful admission: the new encounter, the
/// admission record, and the active bed assignment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AdmissionResult {
    pub encounter: Encounter,
    pub admission: Admission,
    pub bed_assignment: BedAssignment,
}

/// Application service for inpatient ADT (admission / discharge / transfer).
pub struct AdtService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
}

impl AdtService {
    /// Construct an ADT service bound to a database connection and event
    /// publisher.
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The admit() service drives the encounter through the
    /// Planned → Arrived → InProgress state machine. The pure transition
    /// logic is verified here; DB-touching behavior is covered by
    /// integration tests.
    #[test]
    fn test_encounter_admitted_status() {
        let mut enc = Encounter::new(Uuid::new_v4(), EncounterClass::Inpatient);
        assert_eq!(enc.status, EncounterStatus::Planned);
        enc.status = enc
            .status
            .try_transition_to(EncounterStatus::Arrived)
            .expect("planned -> arrived");
        assert_eq!(enc.status, EncounterStatus::Arrived);
        enc.status = enc
            .status
            .try_transition_to(EncounterStatus::InProgress)
            .expect("arrived -> in_progress");
        assert_eq!(enc.status, EncounterStatus::InProgress);
    }
}
