//! encounter repository

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::encounter;
use crate::models::encounter::{Encounter, EncounterClass, EncounterStatus};
use crate::{Error, Result};

pub struct EncounterRepository;

impl EncounterRepository {
    pub async fn create<C: Conn>(conn: &C, e: &Encounter) -> Result<Encounter> {
        let am = to_row(e);
        am.insert(conn).await?;
        Ok(e.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Encounter>> {
        let m = conn
            .one_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    pub async fn update<C: Conn>(conn: &C, e: &Encounter) -> Result<Encounter> {
        to_row(e).update_row(conn).await?;
        Ok(e.clone())
    }

    /// Find the most-recent **active ambulatory** encounter for a
    /// patient — i.e. `class IN ('outpatient', 'emergency')` and
    /// `status IN ('arrived', 'in_progress')`. Used by ADT^A06
    /// (change outpatient to inpatient) to locate the existing
    /// encounter that should be reclassified + bedded.
    pub async fn find_latest_active_ambulatory_for_patient<C: Conn>(
        conn: &C,
        person_id: Uuid,
    ) -> Result<Option<Encounter>> {
        let m = conn
            .one_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE person_id = $1 AND deleted_at IS NULL \
                 AND class IN ('outpatient', 'emergency') AND status IN ('arrived', 'in_progress') \
                 ORDER BY created_at DESC LIMIT 1",
                [person_id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    /// Find the most-recent **planned inpatient** encounter for a patient,
    /// if any. Used by ADT^A38 cancel-pre-admit to locate the encounter
    /// that should be cancelled when a pre-admission is undone.
    pub async fn find_latest_planned_inpatient_for_patient<C: Conn>(
        conn: &C,
        person_id: Uuid,
    ) -> Result<Option<Encounter>> {
        let m = conn.one_as::<encounter::Model>(Stmt::new("SELECT * FROM encounters WHERE person_id = $1 AND deleted_at IS NULL AND status = $2 AND class = $3 ORDER BY created_at DESC LIMIT 1", [person_id.into(), "planned".into(), "inpatient".into()])).await?;
        m.map(from_model).transpose()
    }

    pub async fn list_by_patient<C: Conn>(conn: &C, person_id: Uuid) -> Result<Vec<Encounter>> {
        let rows = conn
            .all_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE person_id = $1 AND deleted_at IS NULL",
                [person_id.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Validate `current -> new_status` against the `EncounterStatus` state
    /// machine, then persist. Returns the updated `Encounter`.
    pub async fn set_status<C: Conn>(
        conn: &C,
        id: Uuid,
        new_status: EncounterStatus,
    ) -> Result<Encounter> {
        Self::set_status_if_current(conn, id, new_status, None).await
    }

    /// [`Self::set_status`] that locks the encounter's row first and refuses with `PreconditionFailed` when
    /// it has changed since the caller read it (OFH-10). `expected` `None` skips the comparison.
    pub async fn set_status_if_current<C: Conn>(
        conn: &C,
        id: Uuid,
        new_status: EncounterStatus,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Encounter> {
        let m = conn
            .one_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("encounter {id}")))?;
        if !crate::api::rest::concurrency::is_current(
            m.updated_at.with_timezone(&chrono::Utc),
            expected,
        ) {
            return Err(Error::precondition_failed(
                "the encounter has changed since it was read; read it again",
            ));
        }
        let current = encounter_status_from_str(&m.status)?;
        current.try_transition_to(new_status)?;
        let mut am = m.patch();
        am.set("status", encounter_status_to_str(new_status).to_string());
        am.set("updated_at", chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await?;
        from_model(updated)
    }

    /// Reclassify an encounter (e.g. Outpatient → Inpatient for
    /// ADT^A06, or Inpatient → Outpatient for ADT^A07). The
    /// EncounterClass enum has no state-machine constraint — any
    /// class change is administratively legal — so this is a plain
    /// field update.
    pub async fn set_class<C: Conn>(
        conn: &C,
        id: Uuid,
        new_class: EncounterClass,
    ) -> Result<Encounter> {
        let m = conn
            .one_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("encounter {id}")))?;
        let mut am = m.patch();
        am.set("class", encounter_class_to_str(new_class).to_string());
        am.set("updated_at", chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await?;
        from_model(updated)
    }

    /// Force the encounter to a target status, bypassing the
    /// [`EncounterStatus`] state machine. **Use only for cancellation
    /// flows** (HL7 v2 ADT^A13 cancel-discharge), which by construction
    /// need to move out of the otherwise-terminal `Finished` state. Every
    /// other write path must go through [`Self::set_status`].
    pub async fn set_status_unchecked<C: Conn>(
        conn: &C,
        id: Uuid,
        new_status: EncounterStatus,
    ) -> Result<Encounter> {
        let m = conn
            .one_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("encounter {id}")))?;
        let mut am = m.patch();
        am.set("status", encounter_status_to_str(new_status).to_string());
        am.set("updated_at", chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await?;
        from_model(updated)
    }
}

// --- conversion helpers ---

pub(crate) fn encounter_class_to_str(c: EncounterClass) -> &'static str {
    match c {
        EncounterClass::Outpatient => "outpatient",
        EncounterClass::Inpatient => "inpatient",
        EncounterClass::Emergency => "emergency",
        EncounterClass::DayCase => "day_case",
        EncounterClass::HomeCare => "home_care",
        EncounterClass::Virtual => "virtual",
    }
}

pub(crate) fn encounter_class_from_str(s: &str) -> Result<EncounterClass> {
    match s {
        "outpatient" => Ok(EncounterClass::Outpatient),
        "inpatient" => Ok(EncounterClass::Inpatient),
        "emergency" => Ok(EncounterClass::Emergency),
        "day_case" => Ok(EncounterClass::DayCase),
        "home_care" => Ok(EncounterClass::HomeCare),
        "virtual" => Ok(EncounterClass::Virtual),
        other => Err(Error::internal(format!("unknown encounter class: {other}"))),
    }
}

pub(crate) fn encounter_status_to_str(s: EncounterStatus) -> &'static str {
    match s {
        EncounterStatus::Planned => "planned",
        EncounterStatus::Arrived => "arrived",
        EncounterStatus::InProgress => "in_progress",
        EncounterStatus::OnLeave => "on_leave",
        EncounterStatus::Finished => "finished",
        EncounterStatus::Cancelled => "cancelled",
    }
}

pub(crate) fn encounter_status_from_str(s: &str) -> Result<EncounterStatus> {
    match s {
        "planned" => Ok(EncounterStatus::Planned),
        "arrived" => Ok(EncounterStatus::Arrived),
        "in_progress" => Ok(EncounterStatus::InProgress),
        "on_leave" => Ok(EncounterStatus::OnLeave),
        "finished" => Ok(EncounterStatus::Finished),
        "cancelled" => Ok(EncounterStatus::Cancelled),
        other => Err(Error::internal(format!(
            "unknown encounter status: {other}"
        ))),
    }
}

fn to_row(e: &Encounter) -> encounter::Model {
    encounter::Model {
        id: e.id,
        person_id: e.person_id,
        class: encounter_class_to_str(e.class).to_string(),
        status: encounter_status_to_str(e.status).to_string(),
        period_start: e.period_start.fixed_offset(),
        period_end: e.period_end.map(|t| t.fixed_offset()),
        worker_id: e.worker_id,
        department_id: e.department_id,
        reason: e.reason.clone(),
        deleted_at: None,
        created_at: e.created_at.fixed_offset(),
        updated_at: e.updated_at.fixed_offset(),
    }
}

fn from_model(m: encounter::Model) -> Result<Encounter> {
    Ok(Encounter {
        id: m.id,
        person_id: m.person_id,
        class: encounter_class_from_str(&m.class)?,
        status: encounter_status_from_str(&m.status)?,
        period_start: m.period_start.with_timezone(&chrono::Utc),
        period_end: m.period_end.map(|t| t.with_timezone(&chrono::Utc)),
        worker_id: m.worker_id,
        department_id: m.department_id,
        reason: m.reason,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encounter_roundtrip_via_row() {
        let e = Encounter::new(Uuid::new_v4(), EncounterClass::Inpatient);
        let am = to_row(&e);
        let m = encounter::Model {
            id: am.id,
            person_id: am.person_id,
            class: am.class.clone(),
            status: am.status.clone(),
            period_start: am.period_start,
            period_end: am.period_end,
            worker_id: am.worker_id,
            department_id: am.department_id,
            reason: am.reason.clone(),
            deleted_at: am.deleted_at,
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, e.id);
        assert_eq!(back.class, EncounterClass::Inpatient);
        assert_eq!(back.status, EncounterStatus::Planned);
    }
}
