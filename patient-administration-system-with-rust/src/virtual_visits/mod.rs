//! Video and phone consultations (CAP-14).
//!
//! A patient who cannot travel can reply `convert` (write-back, CAP-2) to turn an appointment into a video
//! or phone consultation. The appointment keeps its slot and clinician; only its modality changes. The
//! rules:
//!
//! - **A clinician decides which services can go virtual**, not the patient or the PAS. A service is
//!   `virtual_ok` only when someone has approved it by name (`approved_by`); a convert reply on any other
//!   service is refused and the slot is untouched.
//! - **A clinician can veto** before the slot is confirmed as virtual. A conversion starts as
//!   `pending_review`; `confirm` makes it final, `veto` restores in-person.
//! - **The reason is measured**: the barrier that led to it (`cannot_travel`, `caring`, ...) is stored on the
//!   appointment, apart from any cancellation barrier.
//! - It is idempotent by message id (the write-back guarantees that) and audited: the audit row names who or
//!   what converted it.
//!
//! **If a video visit fails to connect** ([`VirtualVisitService::fall_back_to_phone`]), the appointment becomes a
//! phone consultation and a call task (`video_failed`) is put on the staff list, so the patient is phoned
//! rather than the visit lost. Whoever knows it failed (the provider's callback, the clinician or the front
//! desk) calls the endpoint; it is idempotent.
//!
//! Not built here: the video-visit provider and its join link (an interface to NHS Attend Anywhere or the
//! Wales equivalent). Until one is connected nothing reports a failure by itself; a person does.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::models::appointment::{Appointment, ConversionState, Modality};
use crate::{Error, Result};

/// Is the service of the slot's schedule one a clinician has approved for conversion?
pub async fn service_is_virtual_ok<C: ConnectionTrait>(
    conn: &C,
    slot_id: Option<Uuid>,
) -> Result<bool> {
    let Some(slot_id) = slot_id else {
        return Ok(false);
    };
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT 1 AS ok FROM slots sl JOIN schedules sc ON sc.id = sl.schedule_id \
             JOIN virtual_ok_services v ON v.service_type = sc.service_type WHERE sl.id = $1",
            [slot_id.into()],
        ))
        .await
        .map_err(Error::Database)?;
    Ok(row.is_some())
}

#[derive(Debug, Clone, Serialize)]
pub struct ApprovedService {
    pub service_type: String,
    pub approved_by: String,
    pub approved_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Review {
    Confirm,
    Veto,
}

impl Review {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "confirm" => Some(Self::Confirm),
            "veto" => Some(Self::Veto),
            _ => None,
        }
    }
}

pub struct VirtualVisitService {
    db: DatabaseConnection,
}

impl VirtualVisitService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Approve a service for conversion. `approved_by` must name a person; an empty name is refused.
    pub async fn approve_service(
        &self,
        service_type: &str,
        approved_by: &str,
        ctx: &UserContext,
    ) -> Result<ApprovedService> {
        let service_type = service_type.trim();
        let approved_by = approved_by.trim();
        if service_type.is_empty() || service_type.len() > 200 {
            return Err(Error::validation(
                "service_type must be 1 to 200 characters",
            ));
        }
        if approved_by.is_empty() {
            return Err(Error::validation(
                "a service can go virtual only when a named clinician approves it (approved_by)",
            ));
        }
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO virtual_ok_services (service_type, approved_by) VALUES ($1,$2) \
                 ON CONFLICT (service_type) DO UPDATE SET approved_by = $2, approved_at = now()",
                [service_type.into(), approved_by.into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "virtual_ok_service",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, service_type.as_bytes()),
            "approved",
            None,
            Some(serde_json::json!({ "service_type": service_type, "approved_by": approved_by })),
            ctx,
        )
        .await?;
        self.list_services()
            .await?
            .into_iter()
            .find(|s| s.service_type == service_type)
            .ok_or_else(|| Error::internal("approved service not found"))
    }

    pub async fn remove_service(&self, service_type: &str, ctx: &UserContext) -> Result<()> {
        let r = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "DELETE FROM virtual_ok_services WHERE service_type = $1",
                [service_type.trim().into()],
            ))
            .await
            .map_err(Error::Database)?;
        if r.rows_affected() == 0 {
            return Err(Error::not_found(format!(
                "virtual_ok service {service_type}"
            )));
        }
        AuditLogRepository::log(
            &self.db,
            "virtual_ok_service",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, service_type.trim().as_bytes()),
            "removed",
            None,
            Some(serde_json::json!({ "service_type": service_type.trim() })),
            ctx,
        )
        .await
    }

    pub async fn list_services(&self) -> Result<Vec<ApprovedService>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT service_type, approved_by, approved_at FROM virtual_ok_services ORDER BY service_type",
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(ApprovedService {
                    service_type: r.try_get("", "service_type").map_err(Error::Database)?,
                    approved_by: r.try_get("", "approved_by").map_err(Error::Database)?,
                    approved_at: r.try_get("", "approved_at").map_err(Error::Database)?,
                })
            })
            .collect()
    }

    /// Conversions waiting for a clinician, soonest appointment first.
    pub async fn pending(&self, limit: u64) -> Result<Vec<Appointment>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id FROM appointments WHERE conversion_state = 'pending_review' \
                 AND status = 'booked' AND deleted_at IS NULL AND start_datetime > now() \
                 ORDER BY start_datetime LIMIT $1",
                [(limit.min(500) as i64).into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            let id: Uuid = r.try_get("", "id").map_err(Error::Database)?;
            if let Some(a) = AppointmentRepository::find_by_id(&self.db, id).await? {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// A clinician confirms a conversion, or vetoes it and restores in-person. Only a pending conversion
    /// of an appointment that has not started can be reviewed.
    pub async fn review(
        &self,
        appointment_id: Uuid,
        decision: Review,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx = ctx.clone();
        self.db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let mut a = AppointmentRepository::find_by_id(txn, appointment_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
                    if a.conversion_state != Some(ConversionState::PendingReview) {
                        return Err(Error::conflict(
                            "that appointment has no conversion waiting for review",
                        ));
                    }
                    if a.start_datetime <= chrono::Utc::now() {
                        return Err(Error::conflict("that appointment has already started"));
                    }
                    let was = a.modality;
                    match decision {
                        Review::Confirm => a.conversion_state = Some(ConversionState::Confirmed),
                        Review::Veto => {
                            a.conversion_state = Some(ConversionState::Vetoed);
                            a.modality = Modality::InPerson;
                        }
                    }
                    a.updated_at = chrono::Utc::now();
                    let a = AppointmentRepository::update(txn, &a).await?;
                    let verb = if decision == Review::Confirm { "confirmed" } else { "vetoed" };
                    AuditLogRepository::log(
                        txn,
                        "appointment",
                        a.id,
                        &format!("conversion_{verb}"),
                        Some(serde_json::json!({ "modality": was.as_str() })),
                        Some(serde_json::json!({ "modality": a.modality.as_str() })),
                        &ctx,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        if decision == Review::Confirm {
                            "AppointmentConversionConfirmed"
                        } else {
                            "AppointmentConversionVetoed"
                        },
                        &serde_json::json!({ "appointment_id": a.id, "modality": a.modality.as_str() }),
                    )
                    .await?;
                    Ok(a)
                })
            })
            .await
            .map_err(Error::from)
    }

    /// The video visit did not connect: make it a phone consultation and ask a person to ring the patient.
    /// Only a video appointment that is still booked or arrived and has not ended can fall back. Idempotent:
    /// a second report changes nothing and adds no second task.
    pub async fn fall_back_to_phone(
        &self,
        appointment_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Appointment> {
        let ctx = ctx.clone();
        self.db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let mut a = AppointmentRepository::find_by_id(txn, appointment_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
                    if a.modality == Modality::Phone {
                        return Ok(a);
                    }
                    if a.modality != Modality::Video {
                        return Err(Error::conflict("only a video appointment can fall back to a phone call"));
                    }
                    if !matches!(
                        a.status,
                        crate::models::appointment::AppointmentStatus::Booked
                            | crate::models::appointment::AppointmentStatus::Arrived
                    ) || a.end_datetime <= chrono::Utc::now()
                    {
                        return Err(Error::conflict("that appointment is not one that is under way or still to come"));
                    }
                    a.modality = Modality::Phone;
                    a.updated_at = chrono::Utc::now();
                    let a = AppointmentRepository::update(txn, &a).await?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) \
                         VALUES ($1,$2,$3,'video_failed', now()) ON CONFLICT (appointment_id, reason) DO NOTHING",
                        [Uuid::new_v4().into(), a.id.into(), a.person_id.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    AuditLogRepository::log(
                        txn,
                        "appointment",
                        a.id,
                        "video_fell_back_to_phone",
                        Some(serde_json::json!({ "modality": "video" })),
                        Some(serde_json::json!({ "modality": "phone" })),
                        &ctx,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "AppointmentModalityChanged",
                        &serde_json::json!({ "appointment_id": a.id, "modality": "phone", "reason": "video_failed" }),
                    )
                    .await?;
                    Ok(a)
                })
            })
            .await
            .map_err(Error::from)
    }
}
