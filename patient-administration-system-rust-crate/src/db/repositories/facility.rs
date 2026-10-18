//! Facility / Ward / Room repository.
//!
//! Restored in v0.44.0 Phase 3b3 to support REST `POST /api/facilities`,
//! `POST /api/wards`, `POST /api/rooms` plus per-id GETs and list-by-parent
//! reads. v0.43.0 had this CRUD inline in `handlers.rs`; that file was
//! deleted in Phase 2, so the path is replumbed through a dedicated
//! repository now.

use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::{facility, room, ward};
use crate::models::{
    Address,
    facility::{Facility, Room, Ward},
};
use crate::{Error, Result};

// ─── Facility ───────────────────────────────────────────────────────────

pub struct FacilityRepository;

impl FacilityRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, f: &Facility) -> Result<Facility> {
        let am = facility::ActiveModel {
            id: Set(f.id),
            name: Set(f.name.clone()),
            code: Set(f.code.clone()),
            address: Set(serde_json::to_value(&f.address).unwrap_or(serde_json::Value::Null)),
            active: Set(f.active),
            created_at: Set(f.created_at.fixed_offset()),
            updated_at: Set(f.updated_at.fixed_offset()),
        };
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(f.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Facility>> {
        let m = facility::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        Ok(m.map(facility_from_model))
    }

    pub async fn list<C: ConnectionTrait>(conn: &C) -> Result<Vec<Facility>> {
        let rows = facility::Entity::find()
            .filter(facility::Column::Active.eq(true))
            .order_by_asc(facility::Column::Name)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        Ok(rows.into_iter().map(facility_from_model).collect())
    }
}

fn facility_from_model(m: facility::Model) -> Facility {
    Facility {
        id: m.id,
        name: m.name,
        code: m.code,
        address: serde_json::from_value(m.address).unwrap_or(Address {
            use_type: None,
            line1: None,
            line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
        }),
        active: m.active,
        created_at: m.created_at.to_utc(),
        updated_at: m.updated_at.to_utc(),
    }
}

// ─── Ward ───────────────────────────────────────────────────────────────

pub struct WardRepository;

impl WardRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, w: &Ward) -> Result<Ward> {
        let am = ward::ActiveModel {
            id: Set(w.id),
            facility_id: Set(w.facility_id),
            name: Set(w.name.clone()),
            code: Set(w.code.clone()),
            capacity: Set(w.capacity as i32),
            active: Set(w.active),
            created_at: Set(w.created_at.fixed_offset()),
            updated_at: Set(w.updated_at.fixed_offset()),
        };
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(w.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Ward>> {
        let m = ward::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        Ok(m.map(ward_from_model))
    }

    pub async fn list_by_facility<C: ConnectionTrait>(
        conn: &C,
        facility_id: Uuid,
    ) -> Result<Vec<Ward>> {
        let rows = ward::Entity::find()
            .filter(ward::Column::FacilityId.eq(facility_id))
            .filter(ward::Column::Active.eq(true))
            .order_by_asc(ward::Column::Name)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        Ok(rows.into_iter().map(ward_from_model).collect())
    }
}

fn ward_from_model(m: ward::Model) -> Ward {
    Ward {
        id: m.id,
        facility_id: m.facility_id,
        name: m.name,
        code: m.code,
        capacity: m.capacity.max(0) as u32,
        active: m.active,
        created_at: m.created_at.to_utc(),
        updated_at: m.updated_at.to_utc(),
    }
}

// ─── Room ───────────────────────────────────────────────────────────────

pub struct RoomRepository;

impl RoomRepository {
    pub async fn create<C: ConnectionTrait>(conn: &C, r: &Room) -> Result<Room> {
        let am = room::ActiveModel {
            id: Set(r.id),
            ward_id: Set(r.ward_id),
            name: Set(r.name.clone()),
            code: Set(r.code.clone()),
            active: Set(r.active),
            created_at: Set(r.created_at.fixed_offset()),
            updated_at: Set(r.updated_at.fixed_offset()),
        };
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(r.clone())
    }

    pub async fn find_by_id<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Option<Room>> {
        let m = room::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        Ok(m.map(room_from_model))
    }

    pub async fn list_by_ward<C: ConnectionTrait>(conn: &C, ward_id: Uuid) -> Result<Vec<Room>> {
        let rows = room::Entity::find()
            .filter(room::Column::WardId.eq(ward_id))
            .filter(room::Column::Active.eq(true))
            .order_by_asc(room::Column::Name)
            .all(conn)
            .await
            .map_err(Error::Database)?;
        Ok(rows.into_iter().map(room_from_model).collect())
    }

    /// Look up a room by its operator-assigned `code` (the second
    /// component of the HL7 v2 PL data type, e.g. `WARD^ROOM^BED`).
    /// Returns the first match, or `None` if the code isn't unique
    /// enough — callers wanting strict uniqueness should add a
    /// `(ward_id, code)` filter.
    pub async fn find_by_code<C: ConnectionTrait>(conn: &C, code: &str) -> Result<Option<Room>> {
        let m = room::Entity::find()
            .filter(room::Column::Code.eq(code))
            .one(conn)
            .await
            .map_err(Error::Database)?;
        Ok(m.map(room_from_model))
    }
}

fn room_from_model(m: room::Model) -> Room {
    Room {
        id: m.id,
        ward_id: m.ward_id,
        name: m.name,
        code: m.code,
        active: m.active,
        created_at: m.created_at.to_utc(),
        updated_at: m.updated_at.to_utc(),
    }
}
