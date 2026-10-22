//! Non-emergency transport booked from a confirmed appointment (CAP-15).
//!
//! When an appointment is confirmed, an eligible patient who has consented gets a ride requested through a
//! transport coordinator's interface ([`TransportClient`]; the default logs and books nothing real). The rules:
//!
//! - **Eligibility is recorded by staff**, with who and on what basis. The PAS never infers it.
//! - **Consent to share the pick-up address is explicit** and its lawful basis recorded. Without it, no
//!   ride is requested and no address is read.
//! - **The address is not stored.** It is read from person-service when the ride is requested and sent to
//!   the provider with the appointment times and a booking reference, and nothing else: no name, no
//!   service, no clinical detail (see [`TransportRequest`]).
//! - **A ride is requested only after confirmation.** Cancelling (or rebooking, which cancels the old
//!   appointment) cancels the ride; the new appointment gets its own ride when it is confirmed.
//! - **A provider failure never undoes the appointment.** The booking is marked `failed` and the patient goes
//!   on the staff call list.
//!
//! Not built: the real provider (none is chosen), and telling the patient the pick-up window in a message.
//! The window is recorded on the booking and readable at `GET /api/appointments/{id}/transport`.

use crate::db::row::RowExt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;
use uuid::Uuid;

use crate::config::env;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::error::LogErr;
use crate::models::appointment::AppointmentStatus;
use crate::upstream::{PersonClient, UserContext as UpstreamUserContext};
use crate::{Error, Result};

/// Everything the provider is told. Deliberately this and nothing more: a test pins the field names.
#[derive(Debug, Clone, Serialize)]
pub struct TransportRequest {
    /// Our booking id; the provider's only handle on the ride, and not the patient's or appointment's id.
    pub reference: Uuid,
    pub pickup_address: String,
    pub destination: String,
    pub appointment_start: DateTime<Utc>,
    pub appointment_end: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TransportConfirmation {
    pub provider_ref: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

#[async_trait]
pub trait TransportClient: Send + Sync {
    async fn book(&self, req: &TransportRequest) -> Result<TransportConfirmation>;
    async fn cancel(&self, provider_ref: &str) -> Result<()>;
}

/// The default: logs that a ride would be requested (never the address) and returns a made-up reference.
pub struct LoggingTransportClient;

#[async_trait]
impl TransportClient for LoggingTransportClient {
    async fn book(&self, req: &TransportRequest) -> Result<TransportConfirmation> {
        tracing::info!(target: "pas::transport", reference = %req.reference, "transport would be requested (no provider configured)");
        Ok(TransportConfirmation {
            provider_ref: format!("stub-{}", req.reference),
            window_start: req.appointment_start - Duration::minutes(90),
            window_end: req.appointment_start - Duration::minutes(60),
        })
    }
    async fn cancel(&self, provider_ref: &str) -> Result<()> {
        tracing::info!(target: "pas::transport", provider_ref, "transport would be cancelled");
        Ok(())
    }
}

/// A pick-up address as one line, from the person-service `addresses` (FHIR-style: `line`, `city`,
/// `postal_code`, or a ready `text`). `None` when there is nothing usable.
pub fn pickup_address(person: &crate::upstream::Person) -> Option<String> {
    let a = person.data.get("addresses")?.as_array()?.first()?;
    if let Some(t) = a
        .get("text")
        .and_then(|v| v.as_str())
        .filter(|t| !t.trim().is_empty())
    {
        return Some(t.trim().to_string());
    }
    let mut parts: Vec<String> = Vec::new();
    match a.get("line") {
        Some(serde_json::Value::Array(l)) => {
            parts.extend(l.iter().filter_map(|v| v.as_str()).map(String::from));
        }
        Some(serde_json::Value::String(s)) => parts.push(s.clone()),
        _ => {}
    }
    for k in ["city", "district", "postal_code", "postalCode"] {
        if let Some(s) = a.get(k).and_then(|v| v.as_str()) {
            parts.push(s.to_string());
        }
    }
    parts.retain(|p| !p.trim().is_empty());
    (!parts.is_empty()).then(|| parts.join(", "))
}

#[derive(Debug, Clone, Serialize)]
pub struct Booking {
    pub id: Uuid,
    pub appointment_id: Uuid,
    pub status: String,
    pub provider_ref: Option<String>,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

pub struct TransportService {
    db: DatabaseConnection,
    client: Arc<dyn TransportClient>,
    person_client: Option<Arc<dyn PersonClient>>,
}

impl TransportService {
    pub fn new(db: DatabaseConnection, client: Arc<dyn TransportClient>) -> Self {
        Self {
            db,
            client,
            person_client: None,
        }
    }

    pub fn with_person_client(&self, p: Arc<dyn PersonClient>) -> Self {
        Self {
            db: self.db.clone(),
            client: self.client.clone(),
            person_client: Some(p),
        }
    }

    pub fn with_client(&self, c: Arc<dyn TransportClient>) -> Self {
        Self {
            db: self.db.clone(),
            client: c,
            person_client: self.person_client.clone(),
        }
    }

    fn destination() -> String {
        env::text("PAS_CLINIC_ADDRESS").unwrap_or_else(|| "the clinic".into())
    }

    fn need(v: &str, what: &str) -> Result<String> {
        let v = v.trim();
        if v.is_empty() {
            return Err(Error::validation(format!("{what} must not be empty")));
        }
        Ok(v.to_string())
    }

    // --- staff and patient decisions ---------------------------------------------------------

    pub async fn set_eligible(
        &self,
        person: Uuid,
        eligible_by: &str,
        basis: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let (by, basis) = (
            Self::need(eligible_by, "eligible_by")?,
            Self::need(basis, "basis")?,
        );
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO transport_eligibility (person_id, eligible_by, basis) VALUES ($1,$2,$3) \
                 ON CONFLICT (person_id) DO UPDATE SET eligible_by = $2, basis = $3, recorded_at = now()",
                [person.into(), by.clone().into(), basis.clone().into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "patient",
            person,
            "transport_eligibility_recorded",
            None,
            Some(serde_json::json!({ "eligible_by": by, "basis": basis })),
            ctx,
        )
        .await
    }

    pub async fn withdraw_eligible(&self, person: Uuid, ctx: &UserContext) -> Result<()> {
        self.delete_row(
            "transport_eligibility",
            "transport_eligibility_withdrawn",
            person,
            ctx,
        )
        .await
    }

    pub async fn record_consent(
        &self,
        person: Uuid,
        recorded_by: &str,
        basis: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let (by, basis) = (
            Self::need(recorded_by, "recorded_by")?,
            Self::need(basis, "basis")?,
        );
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO transport_consents (person_id, recorded_by, basis) VALUES ($1,$2,$3) \
                 ON CONFLICT (person_id) DO UPDATE SET recorded_by = $2, basis = $3, recorded_at = now()",
                [person.into(), by.clone().into(), basis.clone().into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "patient",
            person,
            "transport_consent_recorded",
            None,
            Some(serde_json::json!({ "recorded_by": by, "basis": basis })),
            ctx,
        )
        .await
    }

    pub async fn withdraw_consent(&self, person: Uuid, ctx: &UserContext) -> Result<()> {
        self.delete_row(
            "transport_consents",
            "transport_consent_withdrawn",
            person,
            ctx,
        )
        .await
    }

    async fn delete_row(
        &self,
        table: &str,
        action: &str,
        person: Uuid,
        ctx: &UserContext,
    ) -> Result<()> {
        let r = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!("DELETE FROM {table} WHERE person_id = $1"),
                [person.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if r.rows_affected() == 0 {
            return Err(Error::not_found("nothing recorded for that patient"));
        }
        AuditLogRepository::log(&self.db, "patient", person, action, None, None, ctx).await
    }

    async fn exists(&self, table: &str, person: Uuid) -> Result<bool> {
        Ok(self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!("SELECT 1 AS ok FROM {table} WHERE person_id = $1"),
                [person.into()],
            ))
            .await
            .map_err(Error::Database)?
            .is_some())
    }

    // --- the ride -----------------------------------------------------------------------------

    pub async fn booking_for(&self, appointment_id: Uuid) -> Result<Option<Booking>> {
        let r = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, appointment_id, status, provider_ref, window_start, window_end, last_error \
                 FROM transport_bookings WHERE appointment_id = $1 ORDER BY created_at DESC LIMIT 1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        r.map(|r| {
            Ok(Booking {
                id: r.col("id")?,
                appointment_id: r.col("appointment_id")?,
                status: r.col("status")?,
                provider_ref: r.col("provider_ref")?,
                window_start: r.col("window_start")?,
                window_end: r.col("window_end")?,
                last_error: r.col("last_error")?,
            })
        })
        .transpose()
    }

    async fn call_task(&self, appt: Uuid, person: Uuid) {
        self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) \
                 VALUES ($1,$2,$3,'transport_failed', now()) ON CONFLICT (appointment_id, reason) DO NOTHING",
                [Uuid::new_v4().into(), appt.into(), person.into()],
            ))
            .await
.log_error("call task for a failed transport booking");
    }

    async fn set_status(&self, id: Uuid, status: &str, error: Option<&str>) -> Result<()> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE transport_bookings SET status = $2, last_error = $3, updated_at = now() WHERE id = $1",
                [id.into(), status.into(), error.map(String::from).into()],
            ))
            .await
            .map_err(Error::Database)?;
        Ok(())
    }

    /// An appointment was confirmed: request a ride if the patient is eligible and has consented. Does
    /// nothing, and reads no address, otherwise. A provider failure is recorded, not raised.
    pub async fn on_confirmed(&self, appointment_id: Uuid) -> Result<()> {
        let Some(appt) = AppointmentRepository::find_by_id(&self.db, appointment_id).await? else {
            return Ok(());
        };
        if appt.status != AppointmentStatus::Booked || appt.confirmed_at.is_none() {
            return Ok(());
        }
        if !self.exists("transport_eligibility", appt.person_id).await?
            || !self.exists("transport_consents", appt.person_id).await?
        {
            return Ok(());
        }
        if let Some(b) = self.booking_for(appointment_id).await?
            && matches!(b.status.as_str(), "requested" | "booked")
        {
            return Ok(());
        }
        let Some(people) = &self.person_client else {
            return Ok(());
        };
        let id = Uuid::new_v4();
        // Claim the single active ride first; a second confirmation at once loses on the unique index.
        let claimed = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO transport_bookings (id, appointment_id, person_id, status) VALUES ($1,$2,$3,'requested') \
                 ON CONFLICT DO NOTHING",
                [id.into(), appointment_id.into(), appt.person_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if claimed.rows_affected() == 0 {
            return Ok(());
        }
        let ctx = UpstreamUserContext {
            user_id: Some("system".into()),
            user_ip: None,
            user_agent: None,
        };
        let address = match people.get(appt.person_id, &ctx).await {
            Ok(p) => pickup_address(&p),
            Err(e) => {
                self.set_status(id, "failed", Some(&format!("person lookup failed: {e}")))
                    .await?;
                self.call_task(appointment_id, appt.person_id).await;
                return Ok(());
            }
        };
        let Some(address) = address else {
            self.set_status(id, "failed", Some("no pick-up address on record"))
                .await?;
            self.call_task(appointment_id, appt.person_id).await;
            return Ok(());
        };
        let req = TransportRequest {
            reference: id,
            pickup_address: address,
            destination: Self::destination(),
            appointment_start: appt.start_datetime,
            appointment_end: appt.end_datetime,
        };
        match self.client.book(&req).await {
            Ok(c) => {
                self.db
                    .execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE transport_bookings SET status = 'booked', provider_ref = $2, window_start = $3, \
                         window_end = $4, updated_at = now() WHERE id = $1",
                        [id.into(), c.provider_ref.into(), c.window_start.into(), c.window_end.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                AuditLogRepository::log(
                    &self.db,
                    "appointment",
                    appointment_id,
                    "transport_booked",
                    None,
                    Some(serde_json::json!({ "booking_id": id })),
                    &UserContext::system(),
                )
                .await?;
            }
            Err(e) => {
                // The provider's error text could echo the request; keep only a fixed description.
                tracing::warn!(target: "pas::transport", booking_id = %id, "provider refused the ride: {e}");
                self.set_status(
                    id,
                    "failed",
                    Some("the transport provider could not book the ride"),
                )
                .await?;
                self.call_task(appointment_id, appt.person_id).await;
            }
        }
        Ok(())
    }

    /// An appointment was cancelled or moved: cancel its ride.
    pub async fn on_cancelled(&self, appointment_id: Uuid) -> Result<()> {
        let Some(b) = self.booking_for(appointment_id).await? else {
            return Ok(());
        };
        if b.status != "booked" {
            return Ok(());
        }
        let person: Uuid = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT person_id FROM transport_bookings WHERE id = $1",
                [b.id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::internal("booking vanished"))?
            .col("person_id")?;
        match self
            .client
            .cancel(b.provider_ref.as_deref().unwrap_or_default())
            .await
        {
            Ok(()) => {
                self.set_status(b.id, "cancelled", None).await?;
                AuditLogRepository::log(
                    &self.db,
                    "appointment",
                    appointment_id,
                    "transport_cancelled",
                    None,
                    Some(serde_json::json!({ "booking_id": b.id })),
                    &UserContext::system(),
                )
                .await?;
            }
            Err(e) => {
                tracing::warn!(target: "pas::transport", booking_id = %b.id, "provider would not cancel: {e}");
                self.set_status(
                    b.id,
                    "failed",
                    Some("the transport provider could not cancel the ride"),
                )
                .await?;
                self.call_task(appointment_id, person).await;
            }
        }
        Ok(())
    }
}
