//! appointment repository

use chrono::{DateTime, Utc};
use sea_orm::sea_query::Condition;
use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::appointment;
use crate::models::appointment::{
    Appointment, AppointmentStatus, CancellationReason, classify_release,
};
use crate::{Error, Result};

pub struct AppointmentRepository;

impl AppointmentRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, a: &Appointment) -> Result<Appointment> {
        let am = to_active_model(a);
        // Return the stored row, so derived fields (`release_class`) are always computed.
        let stored = am.insert(conn).await.map_err(Error::Database)?;
        from_model(stored)
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Appointment>> {
        let m = appointment::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(from_model).transpose()
    }

    pub async fn update<C: ConnectionTrait>(conn: &C, a: &Appointment) -> Result<Appointment> {
        let mut am = to_active_model(a);
        am.created_at = NotSet;
        let stored = am.update(conn).await.map_err(Error::Database)?;
        from_model(stored)
    }

    pub async fn list_by_patient<C: ConnectionTrait>(
        conn: &C,
        person_id: Uuid,
    ) -> Result<Vec<Appointment>> {
        let rows = appointment::Entity::find()
            .filter(appointment::Column::PersonId.eq(person_id))
            .filter(appointment::Column::DeletedAt.is_null())
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// All non-deleted appointments (any status) starting inside `[from, to)`,
    /// earliest first; backs the staff calendar.
    pub async fn list_between<C: ConnectionTrait>(
        conn: &C,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Appointment>> {
        let rows = appointment::Entity::find()
            .filter(appointment::Column::StartDatetime.gte(from.fixed_offset()))
            .filter(appointment::Column::StartDatetime.lt(to.fixed_offset()))
            .filter(appointment::Column::DeletedAt.is_null())
            .order_by_asc(appointment::Column::StartDatetime)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Booked appointments whose start falls inside `[from, to)`; the
    /// reminder scheduler's work list.
    pub async fn list_booked_starting_between<C: ConnectionTrait>(
        conn: &C,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Appointment>> {
        let rows = appointment::Entity::find()
            .filter(
                appointment::Column::Status
                    .eq(appointment_status_to_str(AppointmentStatus::Booked)),
            )
            .filter(appointment::Column::StartDatetime.gte(from.fixed_offset()))
            .filter(appointment::Column::StartDatetime.lt(to.fixed_offset()))
            .filter(appointment::Column::DeletedAt.is_null())
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Appointments for a practitioner inside `[start, end)`.
    pub async fn list_by_practitioner<C: ConnectionTrait>(
        conn: &C,
        worker_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Appointment>> {
        let rows = appointment::Entity::find()
            .filter(appointment::Column::WorkerId.eq(worker_id))
            .filter(appointment::Column::DeletedAt.is_null())
            .filter(appointment::Column::StartDatetime.gte(start.fixed_offset()))
            .filter(appointment::Column::StartDatetime.lt(end.fixed_offset()))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Appointments that overlap `[start, end)` for the given patient,
    /// Every appointment that belongs to a given recurring series, ordered
    /// by scheduled start. Soft-deleted rows are excluded.
    pub async fn list_by_series<C: ConnectionTrait>(
        conn: &C,
        series_id: Uuid,
    ) -> Result<Vec<Appointment>> {
        let rows = appointment::Entity::find()
            .filter(appointment::Column::SeriesId.eq(series_id))
            .filter(appointment::Column::DeletedAt.is_null())
            .order_by_asc(appointment::Column::StartDatetime)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Update one appointment row's status + cancellation_reason. Used by
    /// `SeriesService::cancel_series` to flip all future occurrences to
    /// `Cancelled` in one transaction. Idempotent: missing id → `Ok(None)`.
    pub async fn set_status_and_reason<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
        new_status: AppointmentStatus,
        cancellation_reason: Option<CancellationReason>,
    ) -> Result<Option<Appointment>> {
        let m = match appointment::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
        {
            Some(m) => m,
            None => return Ok(None),
        };
        let now = chrono::Utc::now().fixed_offset();
        let mut am: appointment::ActiveModel = m.into();
        am.status = Set(appointment_status_to_str(new_status).to_string());
        if let Some(reason) = cancellation_reason {
            am.cancellation_reason = Set(Some(cancellation_reason_to_str(reason).to_string()));
        }
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        Ok(Some(from_model(updated)?))
    }

    /// excluding terminal-cancelled/no-show statuses. Used to block
    /// double-booking.
    pub async fn find_overlapping_for_patient<C: ConnectionTrait>(
        conn: &C,
        person_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Appointment>> {
        // Overlap: existing.start < new.end AND existing.end > new.start
        let rows = appointment::Entity::find()
            .filter(appointment::Column::PersonId.eq(person_id))
            .filter(appointment::Column::DeletedAt.is_null())
            .filter(
                Condition::all()
                    .add(appointment::Column::StartDatetime.lt(end.fixed_offset()))
                    .add(appointment::Column::EndDatetime.gt(start.fixed_offset())),
            )
            .filter(
                appointment::Column::Status
                    .is_not_in(vec!["cancelled".to_string(), "no_show".to_string()]),
            )
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(from_model).collect()
    }

    /// Same as [`find_overlapping_for_patient`] but excludes the row with
    /// `exclude_id`. Used by the SIU^S13 reschedule path (v0.17) to check
    /// whether the *new* time window collides with any *other* appointment
    /// for the patient — the reschedule itself naturally overlaps the row
    /// being rescheduled, and that's not a conflict.
    pub async fn find_overlapping_for_patient_excluding<C: ConnectionTrait>(
        conn: &C,
        person_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        exclude_id: Uuid,
    ) -> Result<Vec<Appointment>> {
        let rows = appointment::Entity::find()
            .filter(appointment::Column::PersonId.eq(person_id))
            .filter(appointment::Column::Id.ne(exclude_id))
            .filter(appointment::Column::DeletedAt.is_null())
            .filter(
                Condition::all()
                    .add(appointment::Column::StartDatetime.lt(end.fixed_offset()))
                    .add(appointment::Column::EndDatetime.gt(start.fixed_offset())),
            )
            .filter(
                appointment::Column::Status
                    .is_not_in(vec!["cancelled".to_string(), "no_show".to_string()]),
            )
            .all(conn)
            .await
            .map_err(Error::Database)?;
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

fn to_active_model(a: &Appointment) -> appointment::ActiveModel {
    appointment::ActiveModel {
        id: Set(a.id),
        person_id: Set(a.person_id),
        slot_id: Set(a.slot_id),
        worker_id: Set(a.worker_id),
        start_datetime: Set(a.start_datetime.fixed_offset()),
        end_datetime: Set(a.end_datetime.fixed_offset()),
        status: Set(appointment_status_to_str(a.status).to_string()),
        reason: Set(a.reason.clone()),
        from_waitlist_entry_id: Set(a.from_waitlist_entry_id),
        cancellation_reason: Set(a
            .cancellation_reason
            .map(|c| cancellation_reason_to_str(c).to_string())),
        series_id: Set(a.series_id),
        cancelled_at: Set(a.cancelled_at.map(|t| t.fixed_offset())),
        cancelled_by: Set(a.cancelled_by.map(|c| enum_str(&c))),
        barrier_code: Set(a.barrier_code.map(|b| enum_str(&b))),
        notice_days: Set(a.notice_days),
        confirmed_at: Set(a.confirmed_at.map(|t| t.fixed_offset())),
        deleted_at: Set(None),
        created_at: Set(a.created_at.fixed_offset()),
        updated_at: Set(a.updated_at.fixed_offset()),
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
    fn test_appointment_roundtrip_via_active_model() {
        let start = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        let mut a = Appointment::new(Uuid::new_v4(), start, end);
        a.cancellation_reason = Some(CancellationReason::PatientRequest);
        let am = to_active_model(&a);
        let m = appointment::Model {
            id: am.id.clone().unwrap(),
            person_id: am.person_id.clone().unwrap(),
            slot_id: am.slot_id.clone().unwrap(),
            worker_id: am.worker_id.clone().unwrap(),
            start_datetime: am.start_datetime.clone().unwrap(),
            end_datetime: am.end_datetime.clone().unwrap(),
            status: am.status.clone().unwrap(),
            reason: am.reason.clone().unwrap(),
            from_waitlist_entry_id: am.from_waitlist_entry_id.clone().unwrap(),
            cancellation_reason: am.cancellation_reason.clone().unwrap(),
            series_id: am.series_id.clone().unwrap(),
            cancelled_at: am.cancelled_at.clone().unwrap(),
            cancelled_by: am.cancelled_by.clone().unwrap(),
            barrier_code: am.barrier_code.clone().unwrap(),
            notice_days: am.notice_days.clone().unwrap(),
            confirmed_at: am.confirmed_at.clone().unwrap(),
            deleted_at: am.deleted_at.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
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
        let am = to_active_model(&a);
        assert_eq!(
            am.barrier_code.clone().unwrap().as_deref(),
            Some("cannot_travel")
        );
        assert_eq!(am.cancelled_by.clone().unwrap().as_deref(), Some("patient"));
        let m = appointment::Model {
            id: a.id,
            person_id: a.person_id,
            slot_id: None,
            worker_id: None,
            start_datetime: am.start_datetime.clone().unwrap(),
            end_datetime: am.end_datetime.clone().unwrap(),
            status: "cancelled".into(),
            reason: None,
            from_waitlist_entry_id: None,
            cancellation_reason: None,
            series_id: None,
            cancelled_at: am.cancelled_at.clone().unwrap(),
            cancelled_by: Some("patient".into()),
            barrier_code: Some("cannot_travel".into()),
            notice_days: Some(5),
            confirmed_at: None,
            deleted_at: None,
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = from_model(m).unwrap();
        assert_eq!(back.barrier_code, Some(BarrierCode::CannotTravel));
        assert_eq!(back.cancelled_by, Some(CancelledBy::Patient));
        assert_eq!(back.notice_days, Some(5));
        assert_eq!(back.release_class, Some(ReleaseClass::EarlyRelease));
    }
}
