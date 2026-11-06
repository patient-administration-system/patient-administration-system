//! admission repository
//!
//! Covers `admissions`, `transfers`, `discharges`, and `bed_assignments`
//! tables. The orchestration of ADT (lock bed, write rows, flip statuses)
//! lives in `src/adt/`; this module just persists the records.

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::{admission, bed_assignment, discharge, encounter, transfer};
use crate::models::admission::{Admission, BedAssignment, Discharge, Transfer};
use crate::{Error, Result};

pub struct AdmissionRepository;

impl AdmissionRepository {
    pub async fn create<C: Conn>(conn: &C, a: &Admission) -> Result<Admission> {
        let am = admission_to_row(a);
        am.insert(conn).await?;
        Ok(a.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Admission>> {
        let m = conn
            .one_as::<admission::Model>(Stmt::new(
                "SELECT * FROM admissions WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(admission_from_model).transpose()
    }

    /// Lock the admission's row for the rest of the transaction, check it is the version the caller read, and
    /// move its `updated_at` (OFH-1). Every movement of an admission (transfer, discharge, leave, a cancel of
    /// one of them) starts here, so the admission's `updated_at` is a version that moves whenever the admission
    /// does, which no other column of it would. `expected` `None` (the caller sent `*`) skips the comparison.
    /// A movement that then fails rolls the bump back with the rest of the transaction.
    pub async fn lock_and_bump_if_current<C: Conn>(
        conn: &C,
        id: Uuid,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Admission> {
        let m = conn
            .one_as::<admission::Model>(Stmt::new(
                "SELECT * FROM admissions WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("admission {id}")))?;
        let adm = admission_from_model(m)?;
        if !crate::api::rest::concurrency::is_current(adm.updated_at, expected) {
            return Err(Error::precondition_failed(
                "the admission has changed since it was read; read it again",
            ));
        }
        conn.run(Stmt::new(
            "UPDATE admissions SET updated_at = clock_timestamp() WHERE id = $1",
            [id.into()],
        ))
        .await?;
        Self::find_by_id(conn, id)
            .await?
            .ok_or_else(|| Error::not_found(format!("admission {id}")))
    }

    /// Find the currently-active bed assignment for an encounter, if any.
    ///
    /// "Active" means `released_at IS NULL`. There can be at most one active
    /// row per encounter (enforced by application logic — the schema's
    /// partial unique index is on `bed_id`, not `encounter_id`).
    pub async fn find_active_by_encounter<C: Conn>(
        conn: &C,
        encounter_id: Uuid,
    ) -> Result<Option<BedAssignment>> {
        let m = conn.one_as::<bed_assignment::Model>(Stmt::new("SELECT * FROM bed_assignments WHERE encounter_id = $1 AND released_at IS NULL LIMIT 1", [encounter_id.into()])).await?;
        m.map(bed_assignment_from_model).transpose()
    }

    pub async fn create_transfer<C: Conn>(conn: &C, t: &Transfer) -> Result<Transfer> {
        let am = transfer_to_row(t);
        am.insert(conn).await?;
        Ok(t.clone())
    }

    pub async fn create_discharge<C: Conn>(conn: &C, d: &Discharge) -> Result<Discharge> {
        let am = discharge_to_row(d);
        am.insert(conn).await?;
        Ok(d.clone())
    }

    /// Find the currently-open admission for a patient: an admission whose
    /// bed assignment has `released_at IS NULL`. Returns at most one row;
    /// the application invariant is that a patient has at most one open
    /// inpatient admission at any time.
    pub async fn find_open_for_patient<C: Conn>(
        conn: &C,
        person_id: Uuid,
    ) -> Result<Option<Admission>> {
        let encounter_ids: Vec<Uuid> = conn
            .all_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE person_id = $1",
                [person_id.into()],
            ))
            .await?
            .into_iter()
            .map(|e| e.id)
            .collect();
        if encounter_ids.is_empty() {
            return Ok(None);
        }
        // Active bed assignment = no released_at. Take the most recently
        // assigned one if more than one matches (shouldn't happen).
        let active = conn.one_as::<bed_assignment::Model>(Stmt::new("SELECT * FROM bed_assignments WHERE encounter_id = ANY($1) AND released_at IS NULL ORDER BY assigned_at DESC LIMIT 1", [encounter_ids.clone().into()])).await?;
        let Some(active) = active else {
            return Ok(None);
        };
        let m = conn
            .one_as::<admission::Model>(Stmt::new(
                "SELECT * FROM admissions WHERE encounter_id = $1 LIMIT 1",
                [active.encounter_id.into()],
            ))
            .await?;
        m.map(admission_from_model).transpose()
    }

    /// All admissions for a given patient, joined via `encounters`. Returns
    /// the admission rows only — callers can hydrate the encounter
    /// separately if needed.
    pub async fn list_by_patient<C: Conn>(conn: &C, person_id: Uuid) -> Result<Vec<Admission>> {
        // Two queries: fetch the patient's encounter ids, then admissions.
        let encounter_ids: Vec<Uuid> = conn
            .all_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE person_id = $1",
                [person_id.into()],
            ))
            .await?
            .into_iter()
            .map(|e| e.id)
            .collect();
        if encounter_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = conn
            .all_as::<admission::Model>(Stmt::new(
                "SELECT * FROM admissions WHERE encounter_id = ANY($1)",
                [encounter_ids.into()],
            ))
            .await?;
        rows.into_iter().map(admission_from_model).collect()
    }

    /// Insert a new bed_assignment row. Separate from `create` because
    /// admissions and bed_assignments are independent in v0.1.
    pub async fn create_bed_assignment<C: Conn>(
        conn: &C,
        ba: &BedAssignment,
    ) -> Result<BedAssignment> {
        let am = bed_assignment_to_row(ba);
        am.insert(conn).await?;
        Ok(ba.clone())
    }

    /// Mark the given bed_assignment as released (sets `released_at`).
    pub async fn release_bed_assignment<C: Conn>(conn: &C, id: Uuid) -> Result<BedAssignment> {
        let m = conn
            .one_as::<bed_assignment::Model>(Stmt::new(
                "SELECT * FROM bed_assignments WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("bed_assignment {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set("released_at", Some(now));
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        bed_assignment_from_model(updated)
    }

    /// Find the patient's most-recently-discharged admission: walk
    /// `encounters` for the patient, join `discharges`, return the
    /// admission row corresponding to the discharge with the largest
    /// `discharged_at`. Returns `Ok(None)` when the patient has never been
    /// discharged. Used by the HL7 v2 ADT^A13 (cancel discharge) handler
    /// to identify which discharge to undo.
    pub async fn find_most_recently_discharged_for_patient<C: Conn>(
        conn: &C,
        person_id: Uuid,
    ) -> Result<Option<(Admission, Discharge)>> {
        let encounter_ids: Vec<Uuid> = conn
            .all_as::<encounter::Model>(Stmt::new(
                "SELECT * FROM encounters WHERE person_id = $1",
                [person_id.into()],
            ))
            .await?
            .into_iter()
            .map(|e| e.id)
            .collect();
        if encounter_ids.is_empty() {
            return Ok(None);
        }
        let admissions = conn
            .all_as::<admission::Model>(Stmt::new(
                "SELECT * FROM admissions WHERE encounter_id = ANY($1)",
                [encounter_ids.into()],
            ))
            .await?;
        if admissions.is_empty() {
            return Ok(None);
        }
        let admission_ids: Vec<Uuid> = admissions.iter().map(|a| a.id).collect();
        let latest = conn.one_as::<discharge::Model>(Stmt::new("SELECT * FROM discharges WHERE admission_id = ANY($1) ORDER BY discharged_at DESC LIMIT 1", [admission_ids.into()])).await?;
        let Some(latest) = latest else {
            return Ok(None);
        };
        let adm = admissions
            .into_iter()
            .find(|a| a.id == latest.admission_id)
            .ok_or_else(|| Error::not_found(format!("admission {}", latest.admission_id)))?;
        Ok(Some((
            admission_from_model(adm)?,
            discharge_from_model(latest)?,
        )))
    }

    /// Find the most-recent bed_assignment (active or released) for an
    /// encounter. Used by cancel-discharge to identify which bed to
    /// reinstate. Returns `Ok(None)` when the encounter never had a bed.
    pub async fn find_latest_bed_assignment_for_encounter<C: Conn>(
        conn: &C,
        encounter_id: Uuid,
    ) -> Result<Option<BedAssignment>> {
        let m = conn.one_as::<bed_assignment::Model>(Stmt::new("SELECT * FROM bed_assignments WHERE encounter_id = $1 ORDER BY assigned_at DESC LIMIT 1", [encounter_id.into()])).await?;
        m.map(bed_assignment_from_model).transpose()
    }

    /// Find the most-recent transfer row for an admission, if any.
    /// Used by ADT^A12 (cancel transfer) to identify which transfer
    /// to undo (the latest one).
    pub async fn find_latest_transfer_for_admission<C: Conn>(
        conn: &C,
        admission_id: Uuid,
    ) -> Result<Option<Transfer>> {
        let m = conn.one_as::<transfer::Model>(Stmt::new("SELECT * FROM transfers WHERE admission_id = $1 ORDER BY transferred_at DESC LIMIT 1", [admission_id.into()])).await?;
        m.map(transfer_from_model).transpose()
    }

    /// Delete a transfer row by id. Returns the number of rows
    /// deleted (0 if none matched). Used by ADT^A12 (cancel
    /// transfer) — like `discharges`, the `transfers` table is
    /// operational, not append-only, so a wrong transfer is
    /// physically removed when cancelled.
    pub async fn delete_transfer<C: Conn>(conn: &C, transfer_id: Uuid) -> Result<u64> {
        let res = conn
            .run(Stmt::new(
                "DELETE FROM transfers WHERE id = $1",
                [transfer_id.into()],
            ))
            .await?;
        Ok(res.rows_affected())
    }

    /// Delete the discharge row for the given admission. Returns the number
    /// of rows deleted (0 if there was no discharge). Used by ADT^A13
    /// (cancel discharge) — the `discharges` table is operational, not
    /// append-only, so a row that turns out to have been wrong is
    /// physically removed.
    pub async fn delete_discharge<C: Conn>(conn: &C, admission_id: Uuid) -> Result<u64> {
        let res = conn
            .run(Stmt::new(
                "DELETE FROM discharges WHERE admission_id = $1",
                [admission_id.into()],
            ))
            .await?;
        Ok(res.rows_affected())
    }
}

fn discharge_from_model(m: discharge::Model) -> Result<Discharge> {
    Ok(Discharge {
        id: m.id,
        admission_id: m.admission_id,
        discharging_worker_id: m.discharging_worker_id,
        discharged_at: m.discharged_at.with_timezone(&chrono::Utc),
        disposition: m.disposition,
        notes: m.notes,
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

// --- conversion helpers ---

fn admission_to_row(a: &Admission) -> admission::Model {
    admission::Model {
        id: a.id,
        encounter_id: a.encounter_id,
        bed_id: a.bed_id,
        admitting_worker_id: a.admitting_worker_id,
        admitted_at: a.admitted_at.fixed_offset(),
        reason: a.reason.clone(),
        created_at: a.created_at.fixed_offset(),
        updated_at: a.updated_at.fixed_offset(),
    }
}

fn admission_from_model(m: admission::Model) -> Result<Admission> {
    Ok(Admission {
        id: m.id,
        encounter_id: m.encounter_id,
        bed_id: m.bed_id,
        admitting_worker_id: m.admitting_worker_id,
        admitted_at: m.admitted_at.with_timezone(&chrono::Utc),
        reason: m.reason,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn transfer_from_model(m: transfer::Model) -> Result<Transfer> {
    Ok(Transfer {
        id: m.id,
        admission_id: m.admission_id,
        from_bed_id: m.from_bed_id,
        to_bed_id: m.to_bed_id,
        reason: m.reason,
        transferred_at: m.transferred_at.with_timezone(&chrono::Utc),
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

fn transfer_to_row(t: &Transfer) -> transfer::Model {
    transfer::Model {
        id: t.id,
        admission_id: t.admission_id,
        from_bed_id: t.from_bed_id,
        to_bed_id: t.to_bed_id,
        reason: t.reason.clone(),
        transferred_at: t.transferred_at.fixed_offset(),
        created_at: t.created_at.fixed_offset(),
    }
}

fn discharge_to_row(d: &Discharge) -> discharge::Model {
    discharge::Model {
        id: d.id,
        admission_id: d.admission_id,
        discharging_worker_id: d.discharging_worker_id,
        discharged_at: d.discharged_at.fixed_offset(),
        disposition: d.disposition.clone(),
        notes: d.notes.clone(),
        created_at: d.created_at.fixed_offset(),
    }
}

fn bed_assignment_to_row(ba: &BedAssignment) -> bed_assignment::Model {
    bed_assignment::Model {
        id: ba.id,
        encounter_id: ba.encounter_id,
        bed_id: ba.bed_id,
        assigned_at: ba.assigned_at.fixed_offset(),
        released_at: ba.released_at.map(|t| t.fixed_offset()),
        created_at: ba.created_at.fixed_offset(),
        updated_at: ba.updated_at.fixed_offset(),
    }
}

fn bed_assignment_from_model(m: bed_assignment::Model) -> Result<BedAssignment> {
    Ok(BedAssignment {
        id: m.id,
        encounter_id: m.encounter_id,
        bed_id: m.bed_id,
        assigned_at: m.assigned_at.with_timezone(&chrono::Utc),
        released_at: m.released_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admission_roundtrip_via_row() {
        let a = Admission::new(Uuid::new_v4(), Uuid::new_v4());
        let am = admission_to_row(&a);
        let m = admission::Model {
            id: am.id,
            encounter_id: am.encounter_id,
            bed_id: am.bed_id,
            admitting_worker_id: am.admitting_worker_id,
            admitted_at: am.admitted_at,
            reason: am.reason.clone(),
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = admission_from_model(m).expect("from_model");
        assert_eq!(back.id, a.id);
        assert_eq!(back.encounter_id, a.encounter_id);
        assert_eq!(back.bed_id, a.bed_id);
    }
}
