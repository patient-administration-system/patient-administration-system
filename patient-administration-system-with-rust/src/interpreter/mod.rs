//! Interpreting needs and interpreter requests (NEW-5).
//!
//! A patient's communication profile can say what interpreting they need: a spoken language, British Sign
//! Language, or deafblind interpreting. Booking that patient raises one interpreter request for the
//! appointment, in the same transaction as the booking, wherever the booking comes from (staff, a reply
//! link, standby, an offer, a rebook); cancelling the appointment cancels the request. A person books the
//! interpreter (the Wales Interpretation and Translation Service takes bookings by portal, phone or email,
//! and no API was found) and confirms the request here. Staff work from the list of appointments starting
//! soon whose interpreter is not yet confirmed.

use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt, Transact, Txn};
use chrono::{DateTime, Duration, Utc};
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

crate::string_enum! {
    /// Where an interpreter request is, as `interpreter_requests.status` stores it (QUAL-28). A booking
    /// raises it `requested`; a named person confirms it; cancelling the appointment cancels it.
    pub enum InterpreterStatus {
        Requested = "requested",
        Confirmed = "confirmed",
        Cancelled = "cancelled",
    }
}

/// Raise the interpreter request for a just-booked appointment, if the patient needs one. Called from the
/// one place every booking passes through, inside its transaction.
pub async fn on_booked(txn: &Txn, appt: &Appointment) -> Result<()> {
    let need = txn
        .one(Stmt::new(
            "SELECT interpreter_kind, interpreter_language FROM patient_communication_profiles \
             WHERE person_id = $1 AND interpreter_kind IS NOT NULL",
            [appt.person_id.into()],
        ))
        .await?;
    let Some(need) = need else {
        return Ok(());
    };
    let kind: String = need.col("interpreter_kind")?;
    let language: Option<String> = need.col("interpreter_language")?;
    txn.run(Stmt::new(
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
    .await?;
    Ok(())
}

/// Cancel the interpreter request of a cancelled appointment, inside the cancellation's transaction.
pub async fn on_cancelled(txn: &Txn, appointment_id: Uuid) -> Result<()> {
    txn.run(Stmt::new(
        "UPDATE interpreter_requests SET status = 'cancelled', updated_at = now() \
         WHERE appointment_id = $1 AND status <> 'cancelled'",
        [appointment_id.into()],
    ))
    .await?;
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
    db: Pool,
}

impl InterpreterService {
    pub fn new(db: Pool) -> Self {
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
        let kind: Option<InterpreterKind> = kind
            .map(|k| {
                k.parse().map_err(|_| {
                    Error::validation(format!(
                        "interpreter kind must be one of {}, not {k:?}",
                        InterpreterKind::NAMES.join(", ")
                    ))
                })
            })
            .transpose()?;
        match (kind, language) {
            (None, None) | (Some(InterpreterKind::Spoken), Some(_)) => {}
            (Some(InterpreterKind::Spoken), None) => {
                return Err(Error::validation(
                    "a spoken-language interpreter needs a language",
                ));
            }
            (Some(InterpreterKind::Bsl | InterpreterKind::Deafblind), None) => {}
            (Some(InterpreterKind::Bsl | InterpreterKind::Deafblind), Some(_)) => {
                return Err(Error::validation(
                    "BSL and deafblind interpreting take no language",
                ));
            }
            (None, Some(_)) => {
                return Err(Error::validation("a language needs an interpreter kind"));
            }
        }
        let kind = kind.map(InterpreterKind::as_str);
        let txn = self.db.begin_txn().await?;
        txn.run(Stmt::new("INSERT INTO patient_communication_profiles (person_id, interpreter_kind, interpreter_language, updated_by) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (person_id) DO UPDATE SET interpreter_kind = $2, interpreter_language = $3, \
               updated_by = $4, updated_at = now()",
            [person_id.into(), kind.into(), language.into(), by.into()],
        ))
.await?;
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
        let txn = self.db.begin_txn().await?;
        let done = txn
            .run(Stmt::new("UPDATE interpreter_requests SET status = 'confirmed', confirmed_by = $2, confirmed_at = $3, \
                   booking_ref = $4, updated_at = now() WHERE id = $1 AND status = 'requested'",
                [request_id.into(), by.into(), at.into(), booking_ref.map(str::trim).into()],
            ))
.await?;
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
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<UnconfirmedRequest>> {
        let rows = rows.into();
        let rows = self
            .db
            .all(Stmt::new(
                "SELECT r.id, r.appointment_id, r.person_id, r.kind, r.language, a.start_datetime \
                 FROM interpreter_requests r JOIN appointments a ON a.id = r.appointment_id \
                 WHERE r.status = 'requested' AND a.status = 'booked' \
                   AND a.start_datetime >= $1 AND a.start_datetime < $2 \
                 ORDER BY a.start_datetime, r.id LIMIT $3 OFFSET $4",
                [
                    now.into(),
                    (now + within).into(),
                    rows.fetch().into(),
                    rows.offset().into(),
                ],
            ))
            .await?;
        rows.iter()
            .map(|r| {
                Ok(UnconfirmedRequest {
                    request_id: r.col("id")?,
                    appointment_id: r.col("appointment_id")?,
                    person_id: r.col("person_id")?,
                    kind: r.col::<String>("kind")?.parse()?,
                    language: r.col("language")?,
                    starts_at: r.col("start_datetime")?,
                })
            })
            .collect()
    }
}
