//! bed repository
//!
//! Owns the `beds` table. Provides `select_for_update` for the ADT service
//! to lock a bed inside an admit/transfer transaction.

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::{bed, room};
use crate::models::facility::{Bed, BedStatus};
use crate::{Error, Result};

pub struct BedRepository;

impl BedRepository {
    pub async fn create<C: Conn>(conn: &C, b: &Bed) -> Result<Bed> {
        let am = to_row(b);
        am.insert(conn).await?;
        Ok(b.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Bed>> {
        let m = conn
            .one_as::<bed::Model>(Stmt::new(
                "SELECT * FROM beds WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    /// Find the first bed whose `code` matches. Bed codes are administered
    /// per-facility and typically unique within a facility, but this
    /// repository does not enforce uniqueness across facilities — callers
    /// that need a global lookup should use [`Self::find_by_id`].
    pub async fn find_by_code<C: Conn>(conn: &C, code: &str) -> Result<Option<Bed>> {
        let m = conn
            .one_as::<bed::Model>(Stmt::new(
                "SELECT * FROM beds WHERE code = $1 LIMIT 1",
                [code.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    /// Full replace by id (name + room + code). Status updates stay
    /// on the state-machine-protected `update_status` path; this one
    /// is for MFN^M05 master-file changes that touch the location
    /// metadata but not the operational state. Returns `Error::
    /// NotFound` when the row is missing.
    pub async fn update<C: Conn>(conn: &C, b: &Bed) -> Result<Bed> {
        let existing = conn
            .one_as::<bed::Model>(Stmt::new(
                "SELECT * FROM beds WHERE id = $1 LIMIT 1",
                [b.id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("bed {}", b.id)))?;
        let mut am = existing.patch();
        am.set("room_id", b.room_id);
        am.set("name", b.name.clone());
        am.set("code", b.code.clone());
        am.set("updated_at", chrono::Utc::now().fixed_offset());
        am.update(conn).await?;
        Ok(b.clone())
    }

    /// Validate the bed status transition and persist.
    pub async fn update_status<C: Conn>(conn: &C, id: Uuid, new_status: BedStatus) -> Result<Bed> {
        Self::update_status_if_current(conn, id, new_status, None, false).await
    }

    /// [`Self::update_status`] that, with `lock` set, takes the bed's row lock first and refuses with
    /// `PreconditionFailed` when the bed has changed since the caller read it (OFH-10). `expected` `None`
    /// skips the comparison. The lock is the same one `select_for_update` takes, so it also orders this
    /// against an admission or a hold on the same bed.
    pub async fn update_status_if_current<C: Conn>(
        conn: &C,
        id: Uuid,
        new_status: BedStatus,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        lock: bool,
    ) -> Result<Bed> {
        let sql = if lock {
            "SELECT * FROM beds WHERE id = $1 LIMIT 1 FOR UPDATE"
        } else {
            "SELECT * FROM beds WHERE id = $1 LIMIT 1"
        };
        let m = conn
            .one_as::<bed::Model>(Stmt::new(sql, [id.into()]))
            .await?
            .ok_or_else(|| Error::not_found(format!("bed {id}")))?;
        if !crate::api::rest::concurrency::is_current(
            m.updated_at.with_timezone(&chrono::Utc),
            expected,
        ) {
            return Err(Error::precondition_failed(
                "the bed has changed since it was read; read it again",
            ));
        }
        let current = bed_status_from_str(&m.status)?;
        current.try_transition_to(new_status)?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set("status", bed_status_to_str(new_status).to_string());
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        from_model(updated)
    }

    /// List all beds in a ward. Walks rooms → beds.
    pub async fn list_by_ward<C: Conn>(conn: &C, ward_id: Uuid) -> Result<Vec<Bed>> {
        let room_ids: Vec<Uuid> = conn
            .all_as::<room::Model>(Stmt::new(
                "SELECT * FROM rooms WHERE ward_id = $1",
                [ward_id.into()],
            ))
            .await?
            .into_iter()
            .map(|r| r.id)
            .collect();
        if room_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = conn
            .all_as::<bed::Model>(Stmt::new(
                "SELECT * FROM beds WHERE room_id = ANY($1)",
                [room_ids.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    /// `SELECT … FOR UPDATE` on a single bed row. Use inside a
    /// `DatabaseTransaction` so the lock is held until commit.
    pub async fn select_for_update<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Bed>> {
        let m = conn
            .one_as::<bed::Model>(Stmt::new(
                "SELECT * FROM beds WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    /// Force the bed to a target status, bypassing the [`BedStatus`]
    /// state machine. **Use only for cancellation flows** (HL7 v2 ADT^A11,
    /// ADT^A13) where the regular `Occupied → Cleaning → Available` /
    /// `Cleaning → Available` lifecycle does not apply because the event
    /// being cancelled never properly happened. Every other write path must
    /// go through [`Self::update_status`].
    pub async fn set_status_unchecked<C: Conn>(
        conn: &C,
        id: Uuid,
        new_status: BedStatus,
    ) -> Result<Bed> {
        let m = conn
            .one_as::<bed::Model>(Stmt::new(
                "SELECT * FROM beds WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("bed {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set("status", bed_status_to_str(new_status).to_string());
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        from_model(updated)
    }
}

// --- conversion helpers ---

pub(crate) fn bed_status_to_str(s: BedStatus) -> &'static str {
    match s {
        BedStatus::Available => "available",
        BedStatus::Occupied => "occupied",
        BedStatus::Reserved => "reserved",
        BedStatus::OutOfService => "out_of_service",
        BedStatus::Cleaning => "cleaning",
    }
}

pub(crate) fn bed_status_from_str(s: &str) -> Result<BedStatus> {
    match s {
        "available" => Ok(BedStatus::Available),
        "occupied" => Ok(BedStatus::Occupied),
        "reserved" => Ok(BedStatus::Reserved),
        "out_of_service" => Ok(BedStatus::OutOfService),
        "cleaning" => Ok(BedStatus::Cleaning),
        other => Err(Error::internal(format!("unknown bed status: {other}"))),
    }
}

fn to_row(b: &Bed) -> bed::Model {
    bed::Model {
        id: b.id,
        room_id: b.room_id,
        name: b.name.clone(),
        code: b.code.clone(),
        status: bed_status_to_str(b.status).to_string(),
        created_at: b.created_at.fixed_offset(),
        updated_at: b.updated_at.fixed_offset(),
    }
}

fn from_model(m: bed::Model) -> Result<Bed> {
    Ok(Bed {
        id: m.id,
        room_id: m.room_id,
        name: m.name,
        code: m.code,
        status: bed_status_from_str(&m.status)?,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bed_roundtrip_via_row() {
        let b = Bed::new(Uuid::new_v4(), "Bed 1".into(), "B1".into());
        let am = to_row(&b);
        let m = bed::Model {
            id: am.id,
            room_id: am.room_id,
            name: am.name.clone(),
            code: am.code.clone(),
            status: am.status.clone(),
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, b.id);
        assert_eq!(back.status, BedStatus::Available);
        assert_eq!(back.name, "Bed 1");
    }
}
