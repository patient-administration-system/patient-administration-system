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
//! - Not here: the e-RS integration itself (it needs NHS API access); only the data it would carry.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::models::appointment::Appointment;
use crate::models::waitlist::Priority;
use crate::scheduling::SchedulingService;
use crate::waitlist::WaitlistService;
use crate::{Error, Result};

pub const SOURCES: [&str; 3] = ["gp", "e-rs", "internal"];
pub const STATUSES: [&str; 6] = [
    "received",
    "waitlisted",
    "booked",
    "closed",
    "returned_with_advice",
    "redirected",
];
/// Triage outcomes (NEW-11). Both end the referral at this service.
pub const TRIAGE_OUTCOMES: [&str; 2] = ["returned_with_advice", "redirected"];

/// A referral in one of these states cannot be booked, waitlisted, closed or triaged again.
fn is_ended(status: &str) -> bool {
    matches!(status, "booked" | "closed" | "returned_with_advice" | "redirected")
}
const CANDIDATES: i64 = 25;

#[derive(Debug, Clone, Serialize)]
pub struct Referral {
    pub id: Uuid,
    pub person_id: Uuid,
    pub source: String,
    pub external_ref: Option<String>,
    pub target_service: String,
    pub reason: Option<String>,
    pub referring_worker_id: Option<Uuid>,
    pub status: String,
    pub received_at: DateTime<Utc>,
    /// The appointment booked from this referral, if any.
    pub appointment_id: Option<Uuid>,
    /// Set when a consultant triaged the referral (`returned_with_advice` or `redirected`).
    pub triaged_by: Option<String>,
    pub triaged_at: Option<DateTime<Utc>>,
    /// For a redirect: the service it went to and the new referral that carries the patient there.
    pub redirected_to_service: Option<String>,
    pub redirected_referral_id: Option<Uuid>,
}

pub struct NewReferral {
    pub person_id: Uuid,
    pub source: String,
    pub external_ref: Option<String>,
    pub target_service: String,
    pub reason: Option<String>,
    pub referring_worker_id: Option<Uuid>,
    pub received_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Booked {
    pub referral: Referral,
    pub appointment: Appointment,
}

pub struct ReferralService {
    db: DatabaseConnection,
    waitlist: Arc<WaitlistService>,
}

const COLS: &str = "r.id, r.person_id, r.source, r.external_ref, r.target_service, r.reason, \
    r.referring_worker_id, r.status, r.received_at, \
    r.triaged_by, r.triaged_at, r.redirected_to_service, r.redirected_referral_id, \
    (SELECT a.id FROM appointments a WHERE a.referral_id = r.id AND a.deleted_at IS NULL \
       AND a.status NOT IN ('cancelled','no_show') ORDER BY a.created_at DESC LIMIT 1) AS appointment_id";

fn row_to_referral(r: &sea_orm::QueryResult) -> Result<Referral> {
    let g = |e| Error::Database(e);
    Ok(Referral {
        id: r.try_get("", "id").map_err(g)?,
        person_id: r.try_get("", "person_id").map_err(g)?,
        source: r.try_get("", "source").map_err(g)?,
        external_ref: r.try_get("", "external_ref").map_err(g)?,
        target_service: r.try_get("", "target_service").map_err(g)?,
        reason: r.try_get("", "reason").map_err(g)?,
        referring_worker_id: r.try_get("", "referring_worker_id").map_err(g)?,
        status: r.try_get("", "status").map_err(g)?,
        received_at: r.try_get("", "received_at").map_err(g)?,
        appointment_id: r.try_get("", "appointment_id").map_err(g)?,
        triaged_by: r.try_get("", "triaged_by").map_err(g)?,
        triaged_at: r.try_get("", "triaged_at").map_err(g)?,
        redirected_to_service: r.try_get("", "redirected_to_service").map_err(g)?,
        redirected_referral_id: r.try_get("", "redirected_referral_id").map_err(g)?,
    })
}

impl ReferralService {
    pub fn new(db: DatabaseConnection, waitlist: Arc<WaitlistService>) -> Self {
        Self { db, waitlist }
    }

    pub async fn get(&self, id: Uuid) -> Result<Referral> {
        self.db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!("SELECT {COLS} FROM referrals r WHERE r.id = $1"),
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .map(|r| row_to_referral(&r))
            .transpose()?
            .ok_or_else(|| Error::not_found(format!("referral {id}")))
    }

    pub async fn list(
        &self,
        status: Option<&str>,
        service: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Referral>> {
        if let Some(s) = status
            && !STATUSES.contains(&s)
        {
            return Err(Error::validation(format!(
                "status must be one of {}",
                STATUSES.join(", ")
            )));
        }
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!(
                    "SELECT {COLS} FROM referrals r WHERE ($1::text IS NULL OR r.status = $1) \
                     AND ($2::text IS NULL OR r.target_service = $2) ORDER BY r.received_at LIMIT $3"
                ),
                [
                    status.map(String::from).into(),
                    service.map(String::from).into(),
                    limit.clamp(1, 500).into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter().map(row_to_referral).collect()
    }

    /// Receive a referral. Returns it and whether it had already been received.
    pub async fn create(&self, n: NewReferral, ctx: &UserContext) -> Result<(Referral, bool)> {
        if !SOURCES.contains(&n.source.as_str()) {
            return Err(Error::validation("source must be gp, e-rs or internal"));
        }
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
        if n.source == "e-rs" && ext.is_none() {
            return Err(Error::validation(
                "an e-RS referral needs its reference (the UBRN)",
            ));
        }
        if n.reason.as_deref().is_some_and(|r| r.len() > 1000) {
            return Err(Error::validation("reason must be at most 1000 characters"));
        }
        let id = Uuid::new_v4();
        let received = n.received_at.unwrap_or_else(Utc::now);
        let made = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO referrals (id, person_id, referring_worker_id, target_service, reason, received_at, created_at, \
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
            .await
            .map_err(Error::Database)?;
        if made.rows_affected() == 0 {
            // The same (source, external_ref) was received before.
            let r = self
                .db
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    format!(
                        "SELECT {COLS} FROM referrals r WHERE r.source = $1 AND r.external_ref = $2"
                    ),
                    [n.source.into(), ext.into()],
                ))
                .await
                .map_err(Error::Database)?
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
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .query_one_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT person_id, target_service, status FROM referrals WHERE id = $1 FOR UPDATE",
                            [id.into()],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .ok_or_else(|| Error::not_found(format!("referral {id}")))?;
                    let person: Uuid = r.try_get("", "person_id").map_err(Error::Database)?;
                    let service: String = r.try_get("", "target_service").map_err(Error::Database)?;
                    let status: String = r.try_get("", "status").map_err(Error::Database)?;
                    if is_ended(&status) {
                        return Err(Error::conflict(format!("that referral is already {status}")));
                    }
                    let slots = txn
                        .query_all_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT s.id FROM slots s JOIN schedules sc ON sc.id = s.schedule_id \
                             WHERE sc.service_type = $1 AND sc.active AND s.status = 'free' AND s.start_datetime > now() \
                             ORDER BY s.start_datetime, s.id LIMIT $2",
                            [service.clone().into(), CANDIDATES.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    for row in slots {
                        let slot_id: Uuid = row.try_get("", "id").map_err(Error::Database)?;
                        // A slot that cannot be booked for this patient (they have a clash, or someone just
                        // took it) is skipped; the next earliest is tried.
                        match SchedulingService::book_tx(txn, slot_id, person, &ctx).await {
                            Ok(appt) => {
                                txn.execute_raw(Statement::from_sql_and_values(
                                    DbBackend::Postgres,
                                    "UPDATE appointments SET referral_id = $2 WHERE id = $1",
                                    [appt.id.into(), id.into()],
                                ))
                                .await
                                .map_err(Error::Database)?;
                                txn.execute_raw(Statement::from_sql_and_values(
                                    DbBackend::Postgres,
                                    "UPDATE referrals SET status = 'booked', updated_at = now() WHERE id = $1",
                                    [id.into()],
                                ))
                                .await
                                .map_err(Error::Database)?;
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
            .await
            .map_err(Error::from)?;
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
        if is_ended(&r.status) || r.status == "waitlisted" {
            return Err(Error::conflict(format!(
                "that referral is already {}",
                r.status
            )));
        }
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
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE referrals SET status = 'waitlisted', updated_at = now() WHERE id = $1",
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        self.get(id).await
    }

    pub async fn close(
        &self,
        id: Uuid,
        reason: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Referral> {
        let r = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE referrals SET status = 'closed', updated_at = now() \
                 WHERE id = $1 AND status NOT IN ('closed','returned_with_advice','redirected')",
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if r.rows_affected() == 0 {
            let r = self.get(id).await?; // not found, or already ended
            return Err(Error::conflict(format!(
                "that referral is already {}",
                if r.status == "closed" { "closed" } else { &r.status }
            )));
        }
        AuditLogRepository::log(
            &self.db,
            "referral",
            id,
            "closed",
            None,
            Some(serde_json::json!({ "reason": reason })),
            ctx,
        )
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
        if !TRIAGE_OUTCOMES.contains(&outcome) {
            return Err(Error::validation(
                "outcome must be returned_with_advice or redirected",
            ));
        }
        let by = crate::validation::named_person(
            triaged_by,
            "a referral is triaged by a named person (triaged_by)",
        )?
        .to_string();
        let to = match (outcome, redirect_to_service.map(str::trim)) {
            ("redirected", Some(s)) if !s.is_empty() && s.len() <= 200 => Some(s.to_string()),
            ("redirected", _) => {
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
        let (outcome, ctx) = (outcome.to_string(), ctx.clone());
        let new_id = Uuid::new_v4();
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .query_one_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT person_id, target_service, reason, status FROM referrals WHERE id = $1 FOR UPDATE",
                            [id.into()],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .ok_or_else(|| Error::not_found(format!("referral {id}")))?;
                    let status: String = r.try_get("", "status").map_err(Error::Database)?;
                    if status != "received" {
                        return Err(Error::conflict(format!(
                            "only a referral that has just been received can be triaged; this one is {status}"
                        )));
                    }
                    let person: Uuid = r.try_get("", "person_id").map_err(Error::Database)?;
                    let from: String = r.try_get("", "target_service").map_err(Error::Database)?;
                    let reason: Option<String> = r.try_get("", "reason").map_err(Error::Database)?;
                    if to.as_deref() == Some(from.as_str()) {
                        return Err(Error::validation(
                            "a redirect goes to a different service from the one it was sent to",
                        ));
                    }
                    if let Some(service) = &to {
                        txn.execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "INSERT INTO referrals (id, person_id, target_service, reason, received_at, created_at, source, status) \
                             VALUES ($1,$2,$3,$4,now(),now(),'internal','received')",
                            [new_id.into(), person.into(), service.clone().into(), reason.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    }
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE referrals SET status = $2, triaged_by = $3, triaged_at = now(), \
                                redirected_to_service = $4, redirected_referral_id = $5, updated_at = now() \
                         WHERE id = $1",
                        [
                            id.into(),
                            outcome.clone().into(),
                            by.clone().into(),
                            to.clone().into(),
                            to.as_ref().map(|_| new_id).into(),
                        ],
                    ))
                    .await
                    .map_err(Error::Database)?;
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
                        Some(&outcome),
                        None,
                        &ctx,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await
            .map_err(Error::from)?;
        self.get(id).await
    }
}
