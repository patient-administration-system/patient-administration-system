//! appointment repository

use crate::db::sql::{Conn, Stmt, Table};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::entities::appointment;
use crate::models::appointment::{
    Appointment, AppointmentStatus, CancellationReason, classify_release,
};
use crate::{Error, Result};

pub struct AppointmentRepository;

impl AppointmentRepository {
    pub async fn create<C: Conn>(conn: &C, a: &Appointment) -> Result<Appointment> {
        let am = to_row(a);
        // Return the stored row, so derived fields (`release_class`) are always computed.
        let stored = am.insert(conn).await?;
        from_model(stored)
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Appointment>> {
        let m = conn
            .one_as::<appointment::Model>(Stmt::new(
                "SELECT * FROM appointments WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    /// Lock the appointment's row for the rest of the transaction and check it is the version the caller read
    /// (OFH-1). `expected` is the `updated_at` the caller saw; `None` (the caller sent `*`) skips the
    /// comparison but still takes the lock. Compared to the microsecond, the precision the database keeps.
    /// With the lock held a second writer waits, then sees the new `updated_at` and gets `412`.
    pub async fn lock_if_current<C: Conn>(
        conn: &C,
        id: Uuid,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Appointment> {
        let m = conn
            .one_as::<appointment::Model>(Stmt::new(
                "SELECT * FROM appointments WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("appointment {id}")))?;
        let appt = from_model(m)?;
        if !crate::api::rest::concurrency::is_current(appt.updated_at, expected) {
            return Err(Error::precondition_failed(
                "the appointment has changed since it was read; read it again",
            ));
        }
        Ok(appt)
    }

    pub async fn update<C: Conn>(conn: &C, a: &Appointment) -> Result<Appointment> {
        let stored = to_row(a).update_row(conn).await?;
        from_model(stored)
    }

    pub async fn list_by_patient<C: Conn>(conn: &C, person_id: Uuid) -> Result<Vec<Appointment>> {
        let rows = conn
            .all_as::<appointment::Model>(Stmt::new(
                "SELECT * FROM appointments WHERE person_id = $1 AND deleted_at IS NULL",
                [person_id.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    /// All non-deleted appointments (any status) starting inside `[from, to)`,
    /// earliest first; backs the staff calendar.
    pub async fn list_between<C: Conn>(
        conn: &C,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Appointment>> {
        let rows = conn.all_as::<appointment::Model>(Stmt::new("SELECT * FROM appointments WHERE start_datetime >= $1 AND start_datetime < $2 AND deleted_at IS NULL ORDER BY start_datetime", [from.fixed_offset().into(), to.fixed_offset().into()])).await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Booked appointments whose start falls inside `[from, to)`; the
    /// reminder scheduler's work list.
    pub async fn list_booked_starting_between<C: Conn>(
        conn: &C,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Appointment>> {
        let rows = conn.all_as::<appointment::Model>(Stmt::new("SELECT * FROM appointments WHERE status = $1 AND start_datetime >= $2 AND start_datetime < $3 AND deleted_at IS NULL", [appointment_status_to_str(AppointmentStatus::Booked).into(), from.fixed_offset().into(), to.fixed_offset().into()])).await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Appointments for a practitioner inside `[start, end)`.
    pub async fn list_by_practitioner<C: Conn>(
        conn: &C,
        worker_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Appointment>> {
        let rows = conn.all_as::<appointment::Model>(Stmt::new("SELECT * FROM appointments WHERE worker_id = $1 AND deleted_at IS NULL AND start_datetime >= $2 AND start_datetime < $3", [worker_id.into(), start.fixed_offset().into(), end.fixed_offset().into()])).await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Appointments that overlap `[start, end)` for the given patient,
    /// Every appointment that belongs to a given recurring series, ordered
    /// by scheduled start. Soft-deleted rows are excluded.
    pub async fn list_by_series<C: Conn>(conn: &C, series_id: Uuid) -> Result<Vec<Appointment>> {
        let rows = conn.all_as::<appointment::Model>(Stmt::new("SELECT * FROM appointments WHERE series_id = $1 AND deleted_at IS NULL ORDER BY start_datetime", [series_id.into()])).await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Update one appointment row's status + cancellation_reason. Used by
    /// `SeriesService::cancel_series` to flip all future occurrences to
    /// `Cancelled` in one transaction. Idempotent: missing id → `Ok(None)`.
    pub async fn set_status_and_reason<C: Conn>(
        conn: &C,
        id: Uuid,
        new_status: AppointmentStatus,
        cancellation_reason: Option<CancellationReason>,
    ) -> Result<Option<Appointment>> {
        let m = match conn
            .one_as::<appointment::Model>(Stmt::new(
                "SELECT * FROM appointments WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
        {
            Some(m) => m,
            None => return Ok(None),
        };
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set("status", appointment_status_to_str(new_status).to_string());
        if let Some(reason) = cancellation_reason {
            am.set(
                "cancellation_reason",
                Some(cancellation_reason_to_str(reason).to_string()),
            );
        }
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        Ok(Some(from_model(updated)?))
    }

    /// excluding terminal-cancelled/no-show statuses. Used to block
    /// double-booking.
    pub async fn find_overlapping_for_patient<C: Conn>(
        conn: &C,
        person_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Appointment>> {
        // Overlap: existing.start < new.end AND existing.end > new.start
        let rows = conn
            .all_as::<appointment::Model>(Stmt::new(
                "SELECT * FROM appointments WHERE person_id = $1 AND deleted_at IS NULL \
                 AND start_datetime < $2 AND end_datetime > $3 \
                 AND status NOT IN ('cancelled', 'no_show')",
                [person_id.into(), end.into(), start.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Same as [`Self::find_overlapping_for_patient`] but excludes the row with
    /// `exclude_id`. Used by the SIU^S13 reschedule path (v0.17) to check
    /// whether the *new* time window collides with any *other* appointment
    /// for the patient — the reschedule itself naturally overlaps the row
    /// being rescheduled, and that's not a conflict.
    pub async fn find_overlapping_for_patient_excluding<C: Conn>(
        conn: &C,
        person_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        exclude_id: Uuid,
    ) -> Result<Vec<Appointment>> {
        let rows = conn
            .all_as::<appointment::Model>(Stmt::new(
                "SELECT * FROM appointments WHERE person_id = $1 AND id <> $2 AND deleted_at IS NULL \
                 AND start_datetime < $3 AND end_datetime > $4 \
                 AND status NOT IN ('cancelled', 'no_show')",
                [person_id.into(), exclude_id.into(), end.into(), start.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }
}

// --- conversion helpers ---

pub(crate) fn appointment_status_to_str(s: AppointmentStatus) -> &'static str {
    match s {
        AppointmentStatus::Proposed => "proposed",
        AppointmentStatus::Booked => "booked",
        AppointmentStatus::Arrived => "arrived",
        AppointmentStatus::Fulfilled => "fulfilled",
        AppointmentStatus::Cancelled => "cancelled",
        AppointmentStatus::NoShow => "no_show",
    }
}

pub(crate) fn appointment_status_from_str(s: &str) -> Result<AppointmentStatus> {
    match s {
        "proposed" => Ok(AppointmentStatus::Proposed),
        "booked" => Ok(AppointmentStatus::Booked),
        "arrived" => Ok(AppointmentStatus::Arrived),
        "fulfilled" => Ok(AppointmentStatus::Fulfilled),
        "cancelled" => Ok(AppointmentStatus::Cancelled),
        "no_show" => Ok(AppointmentStatus::NoShow),
        other => Err(Error::internal(format!(
            "unknown appointment status: {other}"
        ))),
    }
}

pub(crate) fn cancellation_reason_to_str(c: CancellationReason) -> &'static str {
    match c {
        CancellationReason::PatientRequest => "patient_request",
        CancellationReason::ProviderRequest => "provider_request",
        CancellationReason::NoShow => "no_show",
        CancellationReason::Rescheduled => "rescheduled",
        CancellationReason::Other => "other",
    }
}

pub(crate) fn cancellation_reason_from_str(s: &str) -> Result<CancellationReason> {
    match s {
        "patient_request" => Ok(CancellationReason::PatientRequest),
        "provider_request" => Ok(CancellationReason::ProviderRequest),
        "no_show" => Ok(CancellationReason::NoShow),
        "rescheduled" => Ok(CancellationReason::Rescheduled),
        "other" => Ok(CancellationReason::Other),
        other => Err(Error::internal(format!(
            "unknown cancellation reason: {other}"
        ))),
    }
}

/// The snake_case wire name of a unit-variant enum, as stored in a text column.
fn enum_str<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|j| j.as_str().map(String::from))
        .unwrap_or_default()
}

fn from_enum_str<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|_| Error::internal(format!("unknown stored value: {s}")))
}

fn to_row(a: &Appointment) -> appointment::Model {
    appointment::Model {
        id: a.id,
        person_id: a.person_id,
        slot_id: a.slot_id,
        worker_id: a.worker_id,
        start_datetime: a.start_datetime.fixed_offset(),
        end_datetime: a.end_datetime.fixed_offset(),
        status: appointment_status_to_str(a.status).to_string(),
        reason: a.reason.clone(),
        from_waitlist_entry_id: a.from_waitlist_entry_id,
        cancellation_reason: a
            .cancellation_reason
            .map(|c| cancellation_reason_to_str(c).to_string()),
        series_id: a.series_id,
        cancelled_at: a.cancelled_at.map(|t| t.fixed_offset()),
        cancelled_by: a.cancelled_by.map(|c| enum_str(&c)),
        barrier_code: a.barrier_code.map(|b| enum_str(&b)),
        notice_days: a.notice_days,
        confirmed_at: a.confirmed_at.map(|t| t.fixed_offset()),
        modality: enum_str(&a.modality),
        conversion_state: a.conversion_state.map(|c| enum_str(&c)),
        conversion_barrier_code: a.conversion_barrier_code.map(|b| enum_str(&b)),
        converted_at: a.converted_at.map(|t| t.fixed_offset()),
        arrived_at: a.arrived_at.map(|t| t.fixed_offset()),
        completed_at: a.completed_at.map(|t| t.fixed_offset()),
        current_room: a.current_room.clone(),
        deleted_at: None,
        created_at: a.created_at.fixed_offset(),
        updated_at: a.updated_at.fixed_offset(),
    }
}

fn from_model(m: appointment::Model) -> Result<Appointment> {
    Ok(Appointment {
        id: m.id,
        person_id: m.person_id,
        slot_id: m.slot_id,
        worker_id: m.worker_id,
        start_datetime: m.start_datetime.with_timezone(&chrono::Utc),
        end_datetime: m.end_datetime.with_timezone(&chrono::Utc),
        status: appointment_status_from_str(&m.status)?,
        reason: m.reason,
        from_waitlist_entry_id: m.from_waitlist_entry_id,
        cancellation_reason: m
            .cancellation_reason
            .as_deref()
            .map(cancellation_reason_from_str)
            .transpose()?,
        series_id: m.series_id,
        cancelled_at: m.cancelled_at.map(|t| t.with_timezone(&chrono::Utc)),
        cancelled_by: m.cancelled_by.as_deref().map(from_enum_str).transpose()?,
        barrier_code: m.barrier_code.as_deref().map(from_enum_str).transpose()?,
        notice_days: m.notice_days,
        confirmed_at: m.confirmed_at.map(|t| t.with_timezone(&chrono::Utc)),
        modality: from_enum_str(&m.modality)?,
        conversion_state: m
            .conversion_state
            .as_deref()
            .map(from_enum_str)
            .transpose()?,
        conversion_barrier_code: m
            .conversion_barrier_code
            .as_deref()
            .map(from_enum_str)
            .transpose()?,
        converted_at: m.converted_at.map(|t| t.with_timezone(&chrono::Utc)),
        arrived_at: m.arrived_at.map(|t| t.with_timezone(&chrono::Utc)),
        completed_at: m.completed_at.map(|t| t.with_timezone(&chrono::Utc)),
        current_room: m.current_room,
        // Derived on read, never stored, so it cannot disagree with status and notice.
        release_class: classify_release(appointment_status_from_str(&m.status)?, m.notice_days),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_appointment_roundtrip_via_row() {
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let mut a = Appointment::new(Uuid::new_v4(), start, end);
        a.cancellation_reason = Some(CancellationReason::PatientRequest);
        let am = to_row(&a);
        let m = appointment::Model {
            id: am.id,
            person_id: am.person_id,
            slot_id: am.slot_id,
            worker_id: am.worker_id,
            start_datetime: am.start_datetime,
            end_datetime: am.end_datetime,
            status: am.status.clone(),
            reason: am.reason.clone(),
            from_waitlist_entry_id: am.from_waitlist_entry_id,
            cancellation_reason: am.cancellation_reason.clone(),
            series_id: am.series_id,
            cancelled_at: am.cancelled_at,
            cancelled_by: am.cancelled_by.clone(),
            barrier_code: am.barrier_code.clone(),
            notice_days: am.notice_days,
            confirmed_at: am.confirmed_at,
            modality: am.modality.clone(),
            conversion_state: am.conversion_state.clone(),
            conversion_barrier_code: am.conversion_barrier_code.clone(),
            converted_at: am.converted_at,
            arrived_at: am.arrived_at,
            completed_at: am.completed_at,
            current_room: am.current_room.clone(),
            deleted_at: am.deleted_at,
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, a.id);
        assert_eq!(back.status, AppointmentStatus::Proposed);
        assert_eq!(
            back.cancellation_reason,
            Some(CancellationReason::PatientRequest)
        );
    }

    #[test]
    fn test_outcome_fields_roundtrip_and_release_class_is_derived() {
        use crate::models::appointment::{BarrierCode, CancelledBy, ReleaseClass};
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let mut a = Appointment::new(Uuid::new_v4(), start, start + chrono::Duration::hours(1));
        a.status = AppointmentStatus::Cancelled;
        a.cancelled_at = Some(start - chrono::Duration::days(5));
        a.cancelled_by = Some(CancelledBy::Patient);
        a.barrier_code = Some(BarrierCode::CannotTravel);
        a.notice_days = Some(5);
        let am = to_row(&a);
        assert_eq!(am.barrier_code.clone().as_deref(), Some("cannot_travel"));
        assert_eq!(am.cancelled_by.clone().as_deref(), Some("patient"));
        let m = appointment::Model {
            id: a.id,
            person_id: a.person_id,
            slot_id: None,
            worker_id: None,
            start_datetime: am.start_datetime,
            end_datetime: am.end_datetime,
            status: "cancelled".into(),
            reason: None,
            from_waitlist_entry_id: None,
            cancellation_reason: None,
            series_id: None,
            cancelled_at: am.cancelled_at,
            cancelled_by: Some("patient".into()),
            barrier_code: Some("cannot_travel".into()),
            notice_days: Some(5),
            confirmed_at: None,
            modality: "in_person".into(),
            conversion_state: None,
            conversion_barrier_code: None,
            converted_at: None,
            arrived_at: None,
            completed_at: None,
            current_room: None,
            deleted_at: None,
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = from_model(m).unwrap();
        assert_eq!(back.barrier_code, Some(BarrierCode::CannotTravel));
        assert_eq!(back.cancelled_by, Some(CancelledBy::Patient));
        assert_eq!(back.notice_days, Some(5));
        assert_eq!(back.release_class, Some(ReleaseClass::EarlyRelease));
    }
}
