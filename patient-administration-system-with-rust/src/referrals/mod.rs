//! Referral management (CORE-1).
//!
//! A referral says "this patient needs this service". It can be received (from a GP, from e-RS, or raised
//! internally), put on the waiting list, booked into the earliest free slot of the service, or closed.
//!
//! - **Receiving is idempotent.** The same `(source, external_ref)` (an e-RS UBRN, say) is one referral;
//!   sending it again returns the first with `duplicate: true`.
//! - **`book_first_available` books the earliest free slot** of the target service that is in the future
//!   and does not clash with another appointment the patient has, links the appointment to the referral
//!   (`appointments.referral_id`) and marks the referral `booked`, in one transaction. With nothing free it
//!   changes nothing and says so: the referral stays open.
//! - Not here: an integration with a national electronic referral service (it needs that service's API access); only the data it would carry.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Row, Stmt, Transact};
use jiff::Timestamp;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::models::appointment::Appointment;
use crate::models::waitlist::Priority;
use crate::scheduling::SchedulingService;
use crate::waitlist::WaitlistService;
use crate::{Error, Result};

// The sources, states, legal moves and shapes are domain rules (QUAL-52).
pub use patient_administration_system_domain::referrals::*;

const CANDIDATES: i64 = 25;

/// The referral an appointment was booked from, if any (PCC-1): its id and the sender's own reference.
pub async fn referral_of_appointment(
    db: &Pool,
    appointment_id: Uuid,
) -> Result<Option<(Uuid, Option<String>)>> {
    let row = db
        .one(Stmt::new(
            "SELECT r.id, r.external_ref FROM referrals r \
             JOIN appointments a ON a.referral_id = r.id WHERE a.id = $1",
            [appointment_id.into()],
        ))
        .await?;
    row.map(|r| Ok((r.col("id")?, r.col("external_ref")?)))
        .transpose()
}

pub struct ReferralService {
    db: Pool,
    waitlist: Arc<WaitlistService>,
}

const COLS: &str = "r.id, r.person_id, r.source, r.external_ref, r.target_service, r.reason, \
    r.referring_worker_id, r.status, r.received_at, \
    r.triaged_by, r.triaged_at, r.redirected_to_service, r.redirected_referral_id, r.updated_at, \
    (SELECT a.id FROM appointments a WHERE a.referral_id = r.id AND a.deleted_at IS NULL \
       AND a.status NOT IN ('cancelled','no_show') ORDER BY a.created_at DESC LIMIT 1) AS appointment_id";

fn row_to_referral(r: &Row) -> Result<Referral> {
    Ok(Referral {
        id: r.col("id")?,
        person_id: r.col("person_id")?,
        source: r.col::<String>("source")?.parse()?,
        external_ref: r.col("external_ref")?,
        target_service: r.col("target_service")?,
        reason: r.col("reason")?,
        referring_worker_id: r.col("referring_worker_id")?,
        status: r.col::<String>("status")?.parse()?,
        received_at: r.col("received_at")?,
        appointment_id: r.col("appointment_id")?,
        triaged_by: r.col("triaged_by")?,
        triaged_at: r.col("triaged_at")?,
        redirected_to_service: r.col("redirected_to_service")?,
        redirected_referral_id: r.col("redirected_referral_id")?,
        updated_at: r.col("updated_at")?,
    })
}

impl ReferralService {
    pub fn new(db: Pool, waitlist: Arc<WaitlistService>) -> Self {
        Self { db, waitlist }
    }

    pub async fn get(&self, id: Uuid) -> Result<Referral> {
        self.db
            .one(Stmt::new(
                format!("SELECT {COLS} FROM referrals r WHERE r.id = $1"),
                [id.into()],
            ))
            .await?
            .map(|r| row_to_referral(&r))
            .transpose()?
            .ok_or_else(|| Error::not_found(format!("referral {id}")))
    }

    /// A person's referrals, oldest first (patient enquiry, WPAS-3).
    pub async fn for_person(&self, person_id: Uuid) -> Result<Vec<Referral>> {
        let rows = self
            .db
            .all(Stmt::new(
                format!("SELECT {COLS} FROM referrals r WHERE r.person_id = $1 ORDER BY r.received_at, r.id"),
                [person_id.into()],
            ))
            .await?;
        rows.iter().map(row_to_referral).collect()
    }

    pub async fn list(
        &self,
        status: Option<&str>,
        service: Option<&str>,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<Referral>> {
        self.list_changed(status, service, None, rows).await
    }

    /// [`Self::list`], or with `changed` the referrals changed in that window (TSC-6), oldest change first.
    pub async fn list_changed(
        &self,
        status: Option<&str>,
        service: Option<&str>,
        changed: Option<(jiff::Timestamp, Option<jiff::Timestamp>)>,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<Referral>> {
        let rows = rows.into();
        if let Some(s) = status {
            s.parse::<ReferralStatus>()?;
        }
        let (since, before) = match changed {
            Some((since, before)) => (Some(since), before),
            None => (None, None),
        };
        let order = if since.is_some() {
            "r.updated_at, r.id"
        } else {
            "r.received_at, r.id"
        };
        let rows = self
            .db
            .all(Stmt::new(format!(
                    "SELECT {COLS} FROM referrals r WHERE ($1::text IS NULL OR r.status = $1) \
                     AND ($2::text IS NULL OR r.target_service = $2) \
                     AND ($5::timestamptz IS NULL OR r.updated_at >= $5) AND ($6::timestamptz IS NULL OR r.updated_at < $6) \
                     ORDER BY {order} LIMIT $3 OFFSET $4"
                ),
                [
                    status.map(String::from).into(),
                    service.map(String::from).into(),
                    rows.fetch()?.into(), rows.offset()?.into(),
                    since.into(),
                    before.into(),
                ],
            ))
.await?;
        rows.iter().map(row_to_referral).collect()
    }

    /// Receive a referral. Returns it and whether it had already been received.
    pub async fn create(&self, n: NewReferral, ctx: &UserContext) -> Result<(Referral, bool)> {
        let source: ReferralSource = n
            .source
            .parse()
            .map_err(|_| Error::validation("source must be gp, e-rs, internal or epr"))?;
        let service = n.target_service.trim().to_string();
        if service.is_empty() || service.len() > 200 {
            return Err(Error::validation(
                "target_service must be 1 to 200 characters",
            ));
        }
        let ext = match n.external_ref.as_deref().map(str::trim) {
            Some("") => return Err(Error::validation("external_ref must not be empty")),
            Some(e) if e.len() > 100 || e.chars().any(char::is_control) => {
                return Err(Error::validation(
                    "external_ref must be at most 100 characters with no control characters",
                ));
            }
            Some(e) => Some(e.to_string()),
            None => None,
        };
        if source == ReferralSource::ERs && ext.is_none() {
            return Err(Error::validation(
                "an e-RS referral needs its reference (the UBRN)",
            ));
        }
        // The EPR's own referral id is how the two records are joined, and what makes a retry the same referral.
        if source == ReferralSource::Epr && ext.is_none() {
            return Err(Error::validation(
                "an EPR referral needs the EPR's referral id as external_ref",
            ));
        }
        if n.reason.as_deref().is_some_and(|r| r.len() > 1000) {
            return Err(Error::validation("reason must be at most 1000 characters"));
        }
        let id = Uuid::new_v4();
        let received = n.received_at.unwrap_or_else(jiff::Timestamp::now);
        let made = self
            .db
            .run(Stmt::new("INSERT INTO referrals (id, person_id, referring_worker_id, target_service, reason, received_at, created_at, \
                                        source, external_ref, status) \
                 VALUES ($1,$2,$3,$4,$5,$6,now(),$7,$8,'received') ON CONFLICT DO NOTHING",
                [
                    id.into(),
                    n.person_id.into(),
                    n.referring_worker_id.into(),
                    service.into(),
                    n.reason.clone().into(),
                    received.into(),
                    n.source.clone().into(),
                    ext.clone().into(),
                ],
            ))
.await?;
        if made.rows_affected() == 0 {
            // The same (source, external_ref) was received before.
            let r = self
                .db
                .one(Stmt::new(
                    format!(
                        "SELECT {COLS} FROM referrals r WHERE r.source = $1 AND r.external_ref = $2"
                    ),
                    [n.source.into(), ext.into()],
                ))
                .await?
                .ok_or_else(|| Error::internal("duplicate referral reported but not found"))?;
            let existing = row_to_referral(&r)?;
            if existing.person_id != n.person_id {
                return Err(Error::conflict(
                    "that reference was already received for a different patient",
                ));
            }
            return Ok((existing, true));
        }
        AuditLogRepository::log_with(
            &self.db,
            "referral",
            id,
            "received",
            None,
            Some(serde_json::json!({ "source": n.source, "target_service": n.target_service.trim() })),
            Some(&n.source),
            None,
            ctx,
        )
        .await?;
        Ok((self.get(id).await?, false))
    }

    /// Book the earliest free slot of the target service for this referral's patient.
    pub async fn book_first_available(&self, id: Uuid, ctx: &UserContext) -> Result<Booked> {
        let ctx = ctx.clone();
        let appt = self
            .db
            .in_txn::<_, Appointment>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .one(Stmt::new("SELECT person_id, target_service, status FROM referrals WHERE id = $1 FOR UPDATE",
                            [id.into()],
                        ))
.await?
                        .ok_or_else(|| Error::not_found(format!("referral {id}")))?;
                    let person: Uuid = r.col("person_id")?;
                    let service: String = r.col("target_service")?;
                    let status: ReferralStatus = r.col::<String>("status")?.parse()?;
                    if is_ended(status) {
                        return Err(Error::conflict(format!("that referral is already {status}")));
                    }
                    let slots = txn
                        .all(Stmt::new("SELECT s.id FROM slots s JOIN schedules sc ON sc.id = s.schedule_id \
                             WHERE sc.service_type = $1 AND sc.active AND s.status = 'free' AND s.capacity = 1 AND NOT s.ring_fenced AND s.start_datetime > now() \
                             ORDER BY s.start_datetime, s.id LIMIT $2",
                            [service.clone().into(), CANDIDATES.into()],
                        ))
.await?;
                    for row in slots {
                        let slot_id: Uuid = row.col("id")?;
                        // A slot that cannot be booked for this patient (they have a clash, or someone just
                        // took it) is skipped; the next earliest is tried.
                        match SchedulingService::book_tx(txn, slot_id, person, &ctx).await {
                            Ok(appt) => {
                                txn.run(Stmt::new("UPDATE appointments SET referral_id = $2, updated_at = now() WHERE id = $1",
                                    [appt.id.into(), id.into()],
                                ))
.await?;
                                txn.run(Stmt::new("UPDATE referrals SET status = 'booked', updated_at = now() WHERE id = $1",
                                    [id.into()],
                                ))
.await?;
                                AuditLogRepository::log_with(
                                    txn,
                                    "referral",
                                    id,
                                    "booked_first_available",
                                    None,
                                    Some(serde_json::json!({ "appointment_id": appt.id, "slot_id": slot_id })),
                                    Some("first_available"),
                                    None,
                                    &ctx,
                                )
                                .await?;
                                return Ok(appt);
                            }
                            Err(Error::Conflict(_)) => continue,
                            Err(e) => return Err(e),
                        }
                    }
                    Err(Error::conflict(format!(
                        "no free appointment is available for {service}; the referral stays open"
                    )))
                })
            })
.await?;
        let appt = AppointmentRepository::find_by_id(&self.db, appt.id)
            .await?
            .ok_or_else(|| Error::internal("booked appointment vanished"))?;
        Ok(Booked {
            referral: self.get(id).await?,
            appointment: appt,
        })
    }

    /// Put the patient on the waiting list for the service, linked to this referral.
    pub async fn waitlist(
        &self,
        id: Uuid,
        priority: Priority,
        ctx: &UserContext,
    ) -> Result<Referral> {
        let r = self.get(id).await?;
        if is_ended(r.status) || r.status == ReferralStatus::Waitlisted {
            return Err(Error::conflict(format!(
                "that referral is already {}",
                r.status
            )));
        }
        // An open suspicion may raise the priority, never lower it (FR-220).
        let priority = match crate::suspicions::booking_priority_of_referral(&self.db, id).await? {
            Some(p) if p > priority => p,
            _ => priority,
        };
        self.waitlist
            .add(
                Some(id),
                r.person_id,
                r.target_service.clone(),
                priority,
                ctx,
            )
            .await?;
        self.db
            .run(Stmt::new(
                "UPDATE referrals SET status = 'waitlisted', updated_at = now() WHERE id = $1",
                [id.into()],
            ))
            .await?;
        self.get(id).await
    }

    /// Close a referral. No version is compared; see [`Self::close_if_unchanged`].
    pub async fn close(
        &self,
        id: Uuid,
        reason: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Referral> {
        self.close_if_unchanged(id, reason, None, ctx).await
    }

    /// [`Self::close`] that refuses with `PreconditionFailed` when the referral has changed since the caller read
    /// it (OFH-11). The row is locked, the version compared, then the status moved, in one transaction.
    pub async fn close_if_unchanged(
        &self,
        id: Uuid,
        reason: Option<&str>,
        expected: Option<Timestamp>,
        ctx: &UserContext,
    ) -> Result<Referral> {
        let (reason, ctx) = (reason.map(str::to_string), ctx.clone());
        self.db
            .in_txn::<_, ()>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .one(Stmt::new(
                            "SELECT status, updated_at FROM referrals WHERE id = $1 FOR UPDATE",
                            [id.into()],
                        ))
                        .await?
                        .ok_or_else(|| Error::not_found(format!("referral {id}")))?;
                    let stored: Timestamp = r.col("updated_at")?;
                    if !crate::api::rest::concurrency::is_current(stored, expected) {
                        return Err(Error::precondition_failed(
                            "that referral has changed since you read it; read it again",
                        ));
                    }
                    let status: ReferralStatus = r.col::<String>("status")?.parse()?;
                    // A booked referral can still be closed; one that has ended cannot.
                    if matches!(
                        status,
                        ReferralStatus::Closed
                            | ReferralStatus::ReturnedWithAdvice
                            | ReferralStatus::Redirected
                    ) {
                        return Err(Error::conflict(format!(
                            "that referral is already {status}"
                        )));
                    }
                    txn.run(Stmt::new(
                        "UPDATE referrals SET status = 'closed', updated_at = now() WHERE id = $1",
                        [id.into()],
                    ))
                    .await?;
                    AuditLogRepository::log(
                        txn,
                        "referral",
                        id,
                        "closed",
                        None,
                        Some(serde_json::json!({ "reason": reason })),
                        &ctx,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await?;
        self.get(id).await
    }

    /// A consultant triages a referral that has not been booked or put on the waiting list: it is returned to
    /// the referrer with advice, or redirected to another service. Either way a named person is recorded and the
    /// referral ends here. A redirect creates the new referral in the same transaction, so the patient is never
    /// left without one. The advice itself is clinical and is not stored here.
    pub async fn triage(
        &self,
        id: Uuid,
        outcome: &str,
        triaged_by: &str,
        redirect_to_service: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Referral> {
        self.triage_if_unchanged(id, outcome, triaged_by, redirect_to_service, None, ctx)
            .await
    }

    /// [`Self::triage`] that refuses with `PreconditionFailed` when the referral has changed since the caller read
    /// it (OFH-11). The version is compared under the row lock the triage already takes.
    pub async fn triage_if_unchanged(
        &self,
        id: Uuid,
        outcome: &str,
        triaged_by: &str,
        redirect_to_service: Option<&str>,
        expected: Option<Timestamp>,
        ctx: &UserContext,
    ) -> Result<Referral> {
        let outcome: TriageOutcome = outcome
            .parse()
            .map_err(|_| Error::validation("outcome must be returned_with_advice or redirected"))?;
        let by = crate::validation::named_person(
            triaged_by,
            "a referral is triaged by a named person (triaged_by)",
        )?
        .to_string();
        let to = match (outcome, redirect_to_service.map(str::trim)) {
            (TriageOutcome::Redirected, Some(s)) if !s.is_empty() && s.len() <= 200 => {
                Some(s.to_string())
            }
            (TriageOutcome::Redirected, _) => {
                return Err(Error::validation(
                    "a redirect names the service it goes to (redirect_to_service, 1 to 200 characters)",
                ));
            }
            (_, Some(s)) if !s.is_empty() => {
                return Err(Error::validation(
                    "redirect_to_service is only for a redirect",
                ));
            }
            _ => None,
        };
        let (outcome, ctx) = (outcome.status().as_str(), ctx.clone());
        let new_id = Uuid::new_v4();
        self.db
            .in_txn::<_, ()>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .one(Stmt::new("SELECT person_id, target_service, reason, status, updated_at FROM referrals WHERE id = $1 FOR UPDATE",
                            [id.into()],
                        ))
.await?
                        .ok_or_else(|| Error::not_found(format!("referral {id}")))?;
                    let stored: Timestamp = r.col("updated_at")?;
                    if !crate::api::rest::concurrency::is_current(stored, expected) {
                        return Err(Error::precondition_failed(
                            "that referral has changed since you read it; read it again",
                        ));
                    }
                    let status: ReferralStatus = r.col::<String>("status")?.parse()?;
                    if status != ReferralStatus::Received {
                        return Err(Error::conflict(format!(
                            "only a referral that has just been received can be triaged; this one is {status}"
                        )));
                    }
                    let person: Uuid = r.col("person_id")?;
                    let from: String = r.col("target_service")?;
                    let reason: Option<String> = r.col("reason")?;
                    if to.as_deref() == Some(from.as_str()) {
                        return Err(Error::validation(
                            "a redirect goes to a different service from the one it was sent to",
                        ));
                    }
                    if let Some(service) = &to {
                        txn.run(Stmt::new("INSERT INTO referrals (id, person_id, target_service, reason, received_at, created_at, source, status) \
                             VALUES ($1,$2,$3,$4,now(),now(),'internal','received')",
                            [new_id.into(), person.into(), service.clone().into(), reason.into()],
                        ))
.await?;
                    }
                    txn.run(Stmt::new("UPDATE referrals SET status = $2, triaged_by = $3, triaged_at = now(), \
                                redirected_to_service = $4, redirected_referral_id = $5, updated_at = now() \
                         WHERE id = $1",
                        [
                            id.into(),
                            outcome.into(),
                            by.clone().into(),
                            to.clone().into(),
                            to.as_ref().map(|_| new_id).into(),
                        ],
                    ))
.await?;
                    AuditLogRepository::log_with(
                        txn,
                        "referral",
                        id,
                        "triaged",
                        Some(serde_json::json!({ "status": status })),
                        Some(serde_json::json!({
                            "status": outcome,
                            "triaged_by": by,
                            "redirected_to_service": to,
                            "redirected_referral_id": to.as_ref().map(|_| new_id),
                        })),
                        Some(outcome),
                        None,
                        &ctx,
                    )
                    .await?;
                    Ok(())
                })
            })
.await?;
        self.get(id).await
    }
}
