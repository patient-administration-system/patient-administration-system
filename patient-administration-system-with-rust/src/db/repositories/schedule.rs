//! schedule repository

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::schedule;
use crate::models::schedule::{Schedule, ScheduleOwner};
use crate::{Error, Result};

pub struct ScheduleRepository;

/// String constants used for the polymorphic `owner_kind` column.
pub(crate) mod owner_kind {
    pub const PRACTITIONER: &str = "practitioner";
    pub const BED: &str = "bed";
    pub const ROOM: &str = "room";
    pub const EQUIPMENT: &str = "equipment";
}

impl ScheduleRepository {
    pub async fn create<C: Conn>(conn: &C, s: &Schedule) -> Result<Schedule> {
        let am = to_row(s);
        am.insert(conn).await?;
        Ok(s.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Schedule>> {
        let m = conn
            .one_as::<schedule::Model>(Stmt::new(
                "SELECT * FROM schedules WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    /// List schedules whose `(owner_kind, owner_id)` matches the given owner.
    pub async fn list_by_owner<C: Conn>(
        conn: &C,
        owner_kind: &str,
        owner_id: Uuid,
    ) -> Result<Vec<Schedule>> {
        let rows = conn
            .all_as::<schedule::Model>(Stmt::new(
                "SELECT * FROM schedules WHERE owner_kind = $1 AND owner_id = $2",
                [owner_kind.into(), owner_id.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Full replace by id. Returns `Error::NotFound` when the row is
    /// missing. Used by the FHIR R5 `PUT /fhir/Schedule/{id}`
    /// surface (v0.21).
    pub async fn update<C: Conn>(conn: &C, s: &Schedule) -> Result<Schedule> {
        let existing = conn
            .one_as::<schedule::Model>(Stmt::new(
                "SELECT * FROM schedules WHERE id = $1 LIMIT 1",
                [s.id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("schedule {}", s.id)))?;
        let (kind, owner_id) = owner_to_kind_and_id(&s.owner);
        let mut am = existing.patch();
        am.set("owner_kind", kind.to_string());
        am.set("owner_id", owner_id);
        am.set("service_type", s.service_type.clone());
        am.set("active", s.active);
        am.set("updated_at", chrono::Utc::now().fixed_offset());
        am.update(conn).await?;
        Ok(s.clone())
    }

    /// Hard delete by id. Schedules do **not** carry a `deleted_at`
    /// column — invariant §5.3 restricts soft delete to patients,
    /// encounters, and appointments. Returns `Error::NotFound` when
    /// the row is missing. Used by `DELETE /fhir/Schedule/{id}`
    /// (v0.21).
    pub async fn delete<C: Conn>(conn: &C, id: Uuid) -> Result<()> {
        let r = conn
            .run(Stmt::new(
                "DELETE FROM schedules WHERE id = $1",
                [id.into()],
            ))
            .await?;
        if r.rows_affected() == 0 {
            return Err(Error::not_found(format!("schedule {id}")));
        }
        Ok(())
    }
}

// --- conversion helpers ---

fn owner_to_kind_and_id(owner: &ScheduleOwner) -> (&'static str, Uuid) {
    match owner {
        ScheduleOwner::Practitioner(id) => (owner_kind::PRACTITIONER, *id),
        ScheduleOwner::Bed(id) => (owner_kind::BED, *id),
        ScheduleOwner::Room(id) => (owner_kind::ROOM, *id),
        ScheduleOwner::Equipment(id) => (owner_kind::EQUIPMENT, *id),
    }
}

fn owner_from_kind_and_id(kind: &str, id: Uuid) -> Result<ScheduleOwner> {
    match kind {
        owner_kind::PRACTITIONER => Ok(ScheduleOwner::Practitioner(id)),
        owner_kind::BED => Ok(ScheduleOwner::Bed(id)),
        owner_kind::ROOM => Ok(ScheduleOwner::Room(id)),
        owner_kind::EQUIPMENT => Ok(ScheduleOwner::Equipment(id)),
        other => Err(Error::internal(format!(
            "unknown schedule owner kind: {other}"
        ))),
    }
}

fn to_row(s: &Schedule) -> schedule::Model {
    let (kind, owner_id) = owner_to_kind_and_id(&s.owner);
    schedule::Model {
        id: s.id,
        owner_kind: kind.to_string(),
        owner_id,
        service_type: s.service_type.clone(),
        active: s.active,
        created_at: s.created_at.fixed_offset(),
        updated_at: s.updated_at.fixed_offset(),
    }
}

fn from_model(m: schedule::Model) -> Result<Schedule> {
    Ok(Schedule {
        id: m.id,
        owner: owner_from_kind_and_id(&m.owner_kind, m.owner_id)?,
        service_type: m.service_type,
        active: m.active,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_roundtrip_via_row() {
        let s = Schedule::new(
            ScheduleOwner::Practitioner(Uuid::new_v4()),
            "general".into(),
        );
        let am = to_row(&s);
        let m = schedule::Model {
            id: am.id,
            owner_kind: am.owner_kind.clone(),
            owner_id: am.owner_id,
            service_type: am.service_type.clone(),
            active: am.active,
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, s.id);
        assert_eq!(back.owner, s.owner);
        assert_eq!(back.service_type, "general");
        assert!(back.active);
    }
}
