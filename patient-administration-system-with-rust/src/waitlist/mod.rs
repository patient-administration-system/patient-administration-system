//! waitlist — Waitlist and RTT clock services

use crate::db::sql::{Conn, Pool, Stmt, Table, Transact};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::entities::rtt_pathway;
use crate::db::repositories::{
    audit::{AuditLogRepository, UserContext},
    outbox::OutboxRepository,
    rtt::RttRepository,
    waitlist::WaitlistRepository,
};
use crate::db::row::RowExt;
use crate::error::LogErr;
use crate::models::rtt::{
    RTTClockEvent, RTTEventKind, RTTPathway, RTTStatus, compute_active_weeks,
};
use crate::models::waitlist::{Priority, WaitlistEntry, WaitlistStatus};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

pub struct WaitlistService {
    db: Pool,
    publisher: Arc<dyn EventPublisher>,
}

impl WaitlistService {
    pub fn new(db: Pool, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    pub async fn add(
        &self,
        referral_id: Option<Uuid>,
        person_id: Uuid,
        target_service: String,
        priority: Priority,
        ctx: &UserContext,
    ) -> Result<WaitlistEntry> {
        let ctx_clone = ctx.clone();
        let mut entry = WaitlistEntry::new(person_id, target_service, priority);
        entry.referral_id = referral_id;
        let entry_clone = entry.clone();
        let res = self
            .db
            .in_txn::<_, WaitlistEntry>(|txn| {
                Box::pin(async move {
                    let e = WaitlistRepository::create_entry(txn, &entry_clone).await?;
                    AuditLogRepository::log(
                        txn,
                        "waitlist_entry",
                        e.id,
                        "add",
                        None,
                        Some(patient_administration_system_domain::jsonx::snapshot(&e)),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "WaitlistAdded",
                        &serde_json::json!({
                            "entry_id": e.id, "person_id": person_id,
                        }),
                    )
                    .await?;
                    Ok(e)
                })
            })
            .await?;
        self.publisher
            .publish(DomainEvent::new(
                "WaitlistAdded",
                serde_json::json!({ "entry_id": res.id }),
            ))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    pub async fn remove(&self, entry_id: Uuid, ctx: &UserContext) -> Result<WaitlistEntry> {
        self.remove_if_unchanged(entry_id, None, ctx).await
    }

    /// [`Self::remove`] for a caller that read the entry (OFH-1): `expected` is the `updated_at` it saw. The row
    /// is locked first; a different version is refused with `412`, changing nothing.
    pub async fn remove_if_unchanged(
        &self,
        entry_id: Uuid,
        expected: Option<jiff::Timestamp>,
        ctx: &UserContext,
    ) -> Result<WaitlistEntry> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .in_txn::<_, WaitlistEntry>(|txn| {
                Box::pin(async move {
                    let mut e =
                        WaitlistRepository::lock_entry_if_current(txn, entry_id, expected).await?;
                    e.status = WaitlistStatus::Removed;
                    e.updated_at = jiff::Timestamp::now();
                    let e = WaitlistRepository::update_entry(txn, &e).await?;
                    AuditLogRepository::log(
                        txn,
                        "waitlist_entry",
                        e.id,
                        "remove",
                        None,
                        None,
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "WaitlistRemoved",
                        &serde_json::json!({ "entry_id": e.id }),
                    )
                    .await?;
                    Ok(e)
                })
            })
            .await?;
        Ok(res)
    }

    pub async fn list_by_service(&self, service: &str) -> Result<Vec<WaitlistEntry>> {
        WaitlistRepository::list_by_service(&self.db, service).await
    }
}

pub struct RttService {
    db: Pool,
    publisher: Arc<dyn EventPublisher>,
}

impl RttService {
    pub fn new(db: Pool, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    pub async fn start(
        &self,
        person_id: Uuid,
        target_service: String,
        ctx: &UserContext,
    ) -> Result<RTTPathway> {
        let ctx_clone = ctx.clone();
        let pathway = RTTPathway::new(person_id, target_service);
        let pathway_clone = pathway.clone();
        let res = self
            .db
            .in_txn::<_, RTTPathway>(|txn| {
                Box::pin(async move {
                    let p = RttRepository::create_pathway(txn, &pathway_clone).await?;
                    let ev = RTTClockEvent::started(p.id);
                    RttRepository::create_event(txn, &ev).await?;
                    AuditLogRepository::log(
                        txn,
                        "rtt_pathway",
                        p.id,
                        "start",
                        None,
                        Some(patient_administration_system_domain::jsonx::snapshot(&p)),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "RTTClockStarted",
                        &serde_json::json!({ "pathway_id": p.id }),
                    )
                    .await?;
                    Ok(p)
                })
            })
            .await?;
        self.publisher
            .publish(DomainEvent::new(
                "RTTClockStarted",
                serde_json::json!({ "pathway_id": res.id }),
            ))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    pub async fn pause(
        &self,
        pathway_id: Uuid,
        reason: String,
        expected: Option<jiff::Timestamp>,
        ctx: &UserContext,
    ) -> Result<RTTClockEvent> {
        self.transition(
            pathway_id,
            expected,
            RTTClockEvent::paused(pathway_id, reason),
            "pause",
            "RTTClockPaused",
            ctx,
        )
        .await
    }

    pub async fn resume(
        &self,
        pathway_id: Uuid,
        expected: Option<jiff::Timestamp>,
        ctx: &UserContext,
    ) -> Result<RTTClockEvent> {
        self.transition(
            pathway_id,
            expected,
            RTTClockEvent::resumed(pathway_id),
            "resume",
            "RTTClockResumed",
            ctx,
        )
        .await
    }

    pub async fn stop(
        &self,
        pathway_id: Uuid,
        reason: String,
        expected: Option<jiff::Timestamp>,
        ctx: &UserContext,
    ) -> Result<RTTClockEvent> {
        self.transition(
            pathway_id,
            expected,
            RTTClockEvent::stopped(pathway_id, reason),
            "stop",
            "RTTClockStopped",
            ctx,
        )
        .await
    }

    /// One pathway, with its status and version (`updated_at`, the `ETag` of `GET /api/rtt/{id}`).
    pub async fn get(&self, pathway_id: Uuid) -> Result<RTTPathway> {
        RttRepository::find_pathway_by_id(&self.db, pathway_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("rtt_pathway {pathway_id}")))
    }

    /// Moves the clock (OFH-12). In one transaction it locks the pathway, refuses a stale version (`412`)
    /// and a move [`RTTStatus::may_become`] does not allow (`409`), and only then records the event.
    async fn transition(
        &self,
        pathway_id: Uuid,
        expected: Option<jiff::Timestamp>,
        event: RTTClockEvent,
        action: &'static str,
        event_type: &'static str,
        ctx: &UserContext,
    ) -> Result<RTTClockEvent> {
        let new_status = match event.kind {
            RTTEventKind::Paused => RTTStatus::Paused,
            RTTEventKind::Resumed => RTTStatus::Active,
            RTTEventKind::Stopped => RTTStatus::Stopped,
            RTTEventKind::Started => {
                return Err(Error::internal(
                    "a clock starts with start, not a transition",
                ));
            }
        };
        let ctx_clone = ctx.clone();
        let event_clone = event.clone();
        let res = self
            .db
            .in_txn::<_, RTTClockEvent>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .one(Stmt::new(
                            "SELECT status, updated_at FROM rtt_pathways WHERE id = $1 FOR UPDATE",
                            [pathway_id.into()],
                        ))
                        .await?
                        .ok_or_else(|| Error::not_found(format!("rtt_pathway {pathway_id}")))?;
                    let stored: jiff::Timestamp = r.col("updated_at")?;
                    if !crate::api::rest::concurrency::is_current(stored, expected) {
                        return Err(Error::precondition_failed(
                            "that RTT pathway has changed since you read it; read it again",
                        ));
                    }
                    let current = match r.col::<String>("status")?.as_str() {
                        "active" => RTTStatus::Active,
                        "paused" => RTTStatus::Paused,
                        "stopped" => RTTStatus::Stopped,
                        other => {
                            return Err(Error::internal(format!(
                                "rtt_pathway {pathway_id} has an unknown status {other}"
                            )));
                        }
                    };
                    if !current.may_become(new_status) {
                        return Err(Error::conflict(current.refusal(new_status)));
                    }
                    let e = RttRepository::create_event(txn, &event_clone).await?;
                    let m = txn
                        .one_as::<rtt_pathway::Model>(Stmt::new(
                            "SELECT * FROM rtt_pathways WHERE id = $1 LIMIT 1",
                            [pathway_id.into()],
                        ))
                        .await?
                        .ok_or_else(|| Error::not_found(format!("rtt_pathway {pathway_id}")))?;
                    let status_str = match new_status {
                        RTTStatus::Active => "active",
                        RTTStatus::Paused => "paused",
                        RTTStatus::Stopped => "stopped",
                    };
                    let mut am = m.patch();
                    am.set("status", status_str.to_string());
                    if matches!(new_status, RTTStatus::Stopped) {
                        am.set("stopped_at", Some(jiff::Timestamp::now()));
                    }
                    am.set("updated_at", jiff::Timestamp::now());
                    am.update(txn).await?;
                    AuditLogRepository::log(
                        txn,
                        "rtt_pathway",
                        pathway_id,
                        action,
                        None,
                        Some(patient_administration_system_domain::jsonx::snapshot(&e)),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        event_type,
                        &serde_json::json!({ "pathway_id": pathway_id }),
                    )
                    .await?;
                    Ok(e)
                })
            })
            .await?;
        self.publisher
            .publish(DomainEvent::new(
                event_type,
                serde_json::json!({ "pathway_id": pathway_id }),
            ))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    pub async fn weeks_waiting(&self, pathway_id: Uuid) -> Result<u32> {
        let events = RttRepository::list_events_for_pathway(&self.db, pathway_id).await?;
        Ok(compute_active_weeks(&events, jiff::Timestamp::now()))
    }

    pub async fn is_breaching(&self, pathway_id: Uuid) -> Result<bool> {
        let pathway = RttRepository::find_pathway_by_id(&self.db, pathway_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("rtt_pathway {pathway_id}")))?;
        let events = RttRepository::list_events_for_pathway(&self.db, pathway_id).await?;
        Ok(pathway.is_breaching(&events, jiff::Timestamp::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_weeks_empty() {
        assert_eq!(compute_active_weeks(&[], jiff::Timestamp::now()), 0);
    }
}
