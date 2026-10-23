//! scheduling — Slot/Appointment booking service + recurring series.

pub mod series;

pub use series::{
    CreateSeriesInput, CreateSeriesResult, PreviewResult, SeriesService, SeriesWithOccurrences,
};

use crate::db::row::RowExt;
use chrono::{DateTime, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::waitlist::WaitlistRepository;
use crate::db::repositories::{
    appointment::AppointmentRepository,
    audit::{AuditLogRepository, UserContext},
    outbox::OutboxRepository,
    slot::SlotRepository,
};
use crate::error::LogErr;
use crate::models::appointment::{
    Appointment, AppointmentStatus, BarrierCode, CancellationReason, CancelledBy, notice_days,
};
use crate::models::schedule::SlotStatus;
use crate::models::waitlist::WaitlistStatus;
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

mod attendance;
mod booking;
mod cancelling;
mod events;
pub mod ihe;

/// How a new booking relates to a waiting-list entry (CORE-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryLink {
    /// Not from a waiting list.
    None,
    /// Booking *from* this entry: it must exist, belong to the patient and be waiting.
    Explicit(Uuid),
    /// A rebook carrying the old appointment's entry forward. Lenient: if the entry has moved on
    /// (removed, treated) the new appointment is simply not linked to a live entry.
    Inherit(Uuid),
}

/// What the caller knows about a cancellation (CAP-1).
#[derive(Debug, Clone, Copy)]
pub struct CancelDetails {
    pub cancelled_by: CancelledBy,
    pub barrier_code: Option<BarrierCode>,
    /// Overrides the reason derived from `cancelled_by` (used by `rescheduled`).
    pub legacy_reason: Option<CancellationReason>,
}

pub struct SchedulingService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
    /// The transport service (CAP-15), set after construction because it needs the person client. Shared
    /// by every clone of the `Arc<SchedulingService>`, so swapping it reaches all of them.
    transport: std::sync::RwLock<Option<Arc<crate::transport::TransportService>>>,
    /// The no-show probability (CAP-12), set the same way. Only ever added support, and off unless approved.
    noshow: std::sync::RwLock<Option<Arc<crate::noshow::NoShowService>>>,
    /// The backfill offers (PILOT-8), set the same way.
    backfill: std::sync::RwLock<Option<Arc<crate::backfill::BackfillService>>>,
}

impl SchedulingService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self {
            db,
            publisher,
            transport: std::sync::RwLock::new(None),
            backfill: std::sync::RwLock::new(None),
            noshow: std::sync::RwLock::new(None),
        }
    }

    pub fn set_transport(&self, t: Arc<crate::transport::TransportService>) {
        *self
            .transport
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(t);
    }

    /// A clone out of the lock, so the guard is never held across an await.
    fn noshow(&self) -> Option<Arc<crate::noshow::NoShowService>> {
        self.noshow
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_noshow(&self, n: Arc<crate::noshow::NoShowService>) {
        *self
            .noshow
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(n);
    }

    pub fn set_backfill(&self, b: Arc<crate::backfill::BackfillService>) {
        *self
            .backfill
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(b);
    }

    fn backfill(&self) -> Option<Arc<crate::backfill::BackfillService>> {
        self.backfill
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn transport(&self) -> Option<Arc<crate::transport::TransportService>> {
        self.transport
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// After an appointment has been confirmed: request a ride if the patient is eligible and has
    /// consented (CAP-15). Best effort: the confirmation stands whatever happens.
    pub(crate) async fn after_confirmed(&self, appointment_id: Uuid) {
        if let Some(t) = self.transport()
            && let Err(e) = t.on_confirmed(appointment_id).await
        {
            tracing::warn!(target: "pas::transport", "transport request failed: {e}");
        }
    }

    // --- transaction-level building blocks -------------------------------------------------
    //
    // These run inside a caller-supplied transaction so that larger operations (rebooking, a
    // patient's reply arriving from outside) can combine them atomically. They publish nothing;
    // the caller publishes the returned events after the commit.

    // --- public operations ----------------------------------------------------------------
}

/// The snake_case wire name of a unit-variant enum.
fn enum_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|j| j.as_str().map(String::from))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_appointment_proposed_to_booked() {
        let s = AppointmentStatus::Proposed
            .try_transition_to(AppointmentStatus::Booked)
            .unwrap();
        assert_eq!(s, AppointmentStatus::Booked);
    }

    #[test]
    fn test_slot_free_to_busy() {
        let s = SlotStatus::Free
            .try_transition_to(SlotStatus::Busy)
            .unwrap();
        assert_eq!(s, SlotStatus::Busy);
    }
}
