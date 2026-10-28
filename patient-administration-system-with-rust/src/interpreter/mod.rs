//! Interpreting needs and interpreter requests (NEW-5).
//!
//! A patient's communication profile can say what interpreting they need: a spoken language, British Sign
//! Language, or deafblind interpreting. Booking that patient raises one interpreter request for the
//! appointment, in the same transaction as the booking, wherever the booking comes from (staff, a reply
//! link, standby, an offer, a rebook); cancelling the appointment cancels the request. A person books the
//! interpreter (the Wales Interpretation and Translation Service takes bookings by portal, phone or email,
//! and no API was found) and confirms the request here. Staff work from the list of appointments starting
//! soon whose interpreter is not yet confirmed.

use chrono::{DateTime, Duration, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait,
};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::models::appointment::Appointment;
use crate::validation::named_person;
use crate::{Error, Result};

crate::string_enum! {
    /// The kinds of interpreting a profile can record.
    pub enum InterpreterKind {
        Spoken = "spoken",
        Bsl = "bsl",
        Deafblind = "deafblind",
    }
}

/// Raise the interpreter request for a just-booked appointment, if the patient needs one. Called from the
/// one place every booking passes through, inside its transaction.
pub async fn on_booked(txn: &DatabaseTransaction, appt: &Appointment) -> Result<()> {
    let need = txn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT interpreter_kind, interpreter_language FROM patient_communication_profiles \
             WHERE person_id = $1 AND interpreter_kind IS NOT NULL",
            [appt.person_id.into()],
        ))
        .await
        .map_err(Error::Database)?;
    let Some(need) = need else {
        return Ok(());
    };
    let kind: String = need
        .try_get("", "interpreter_kind")
        .map_err(Error::Database)?;
    let language: Option<String> = need
        .try_get("", "interpreter_language")
        .map_err(Error::Database)?;
    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO interpreter_requests (id, appointment_id, person_id, kind, language) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT (appointment_id) DO NOTHING",
        [
            Uuid::new_v4().into(),
            appt.id.into(),
            appt.person_id.into(),
            kind.into(),
            language.into(),
        ],
    ))
    .await
    .map_err(Error::Database)?;
    Ok(())
}

/// Cancel the interpreter request of a cancelled appointment, inside the cancellation's transaction.
pub async fn on_cancelled(txn: &DatabaseTransaction, appointment_id: Uuid) -> Result<()> {
    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE interpreter_requests SET status = 'cancelled', updated_at = now() \
         WHERE appointment_id = $1 AND status <> 'cancelled'",
        [appointment_id.into()],
    ))
    .await
    .map_err(Error::Database)?;
    Ok(())
}

#[derive(Debug, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct UnconfirmedRequest {
    pub request_id: Uuid,
    pub appointment_id: Uuid,
    pub person_id: Uuid,
    pub kind: InterpreterKind,
    #[schema(required = true)]
    pub language: Option<String>,
    pub starts_at: DateTime<Utc>,
}

pub struct InterpreterService {
    db: DatabaseConnection,
}

impl InterpreterService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Record (or clear, with `kind` `None`) what interpreting a patient needs. A named person; audited.
    pub async fn set_need(
        &self,
        person_id: Uuid,
        kind: Option<&str>,
        language: Option<&str>,
        recorded_by: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            recorded_by,
            "recorded_by: the person recording the need is required",
        )?;
        let language = language.map(str::trim).filter(|l| !l.is_empty());
        match (kind, language) {
            (None, None) => {}
            (Some("spoken"), Some(_)) => {}
            (Some("spoken"), None) => {
                return Err(Error::validation(
                    "a spoken-language interpreter needs a language",
                ));
            }
            (Some("bsl" | "deafblind"), None) => {}
            (Some("bsl" | "deafblind"), Some(_)) => {
                return Err(Error::validation(
                    "BSL and deafblind interpreting take no language",
                ));
            }
            (Some(other), _) => {
                return Err(Error::validation(format!(
                    "interpreter kind must be one of {}, not {other:?}",
                    InterpreterKind::NAMES.join(", ")
                )));
            }
            (None, Some(_)) => {
                return Err(Error::validation("a language needs an interpreter kind"));
            }
        }
        let txn = self.db.begin().await.map_err(Error::Database)?;
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO patient_communication_profiles (person_id, interpreter_kind, interpreter_language, updated_by) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (person_id) DO UPDATE SET interpreter_kind = $2, interpreter_language = $3, \
               updated_by = $4, updated_at = now()",
            [person_id.into(), kind.into(), language.into(), by.into()],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "communication_profile",
            person_id,
            "set_interpreter_need",
            None,
            Some(serde_json::json!({ "interpreter_kind": kind, "interpreter_language": language })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// A named person confirms that the interpreter is booked, with the interpreting service's reference.
    pub async fn confirm(
        &self,
        request_id: Uuid,
        confirmed_by: &str,
        booking_ref: Option<&str>,
        at: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            confirmed_by,
            "confirmed_by: the person confirming the interpreter is required",
        )?;
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let done = txn
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE interpreter_requests SET status = 'confirmed', confirmed_by = $2, confirmed_at = $3, \
                   booking_ref = $4, updated_at = now() WHERE id = $1 AND status = 'requested'",
                [request_id.into(), by.into(), at.into(), booking_ref.map(str::trim).into()],
            ))
            .await
            .map_err(Error::Database)?;
        if done.rows_affected() != 1 {
            return Err(Error::not_found(format!(
                "open interpreter request {request_id}"
            )));
        }
        AuditLogRepository::log(
            &txn,
            "interpreter_request",
            request_id,
            "confirm",
            None,
            Some(serde_json::json!({ "confirmed_by": by, "booking_ref": booking_ref })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// Booked appointments starting between `now` and `now + within` whose interpreter is not confirmed,
    /// soonest first. At most `fetch` rows (see `crate::paging`).
    pub async fn unconfirmed(
        &self,
        now: DateTime<Utc>,
        within: Duration,
        fetch: i64,
    ) -> Result<Vec<UnconfirmedRequest>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT r.id, r.appointment_id, r.person_id, r.kind, r.language, a.start_datetime \
                 FROM interpreter_requests r JOIN appointments a ON a.id = r.appointment_id \
                 WHERE r.status = 'requested' AND a.status = 'booked' \
                   AND a.start_datetime >= $1 AND a.start_datetime < $2 \
                 ORDER BY a.start_datetime, r.id LIMIT $3",
                [
                    now.into(),
                    (now + within).into(),
                    crate::paging::fetch(fetch).into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(UnconfirmedRequest {
                    request_id: r.try_get("", "id").map_err(Error::Database)?,
                    appointment_id: r.try_get("", "appointment_id").map_err(Error::Database)?,
                    person_id: r.try_get("", "person_id").map_err(Error::Database)?,
                    kind: r
                        .try_get::<String>("", "kind")
                        .map_err(Error::Database)?
                        .parse()?,
                    language: r.try_get("", "language").map_err(Error::Database)?,
                    starts_at: r.try_get("", "start_datetime").map_err(Error::Database)?,
                })
            })
            .collect()
    }
}
