//! Facility / Ward / Room repository: the CRUD behind `POST /api/facilities`, `POST /api/wards` and
//! `POST /api/rooms`, the per-id GETs and the list-by-parent reads.

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::Result;
use crate::db::entities::{facility, room, ward};
use crate::models::{
    Address,
    facility::{Facility, Room, Ward},
};

// ─── Facility ───────────────────────────────────────────────────────────

pub struct FacilityRepository;

impl FacilityRepository {
    pub async fn create<C: Conn>(conn: &C, f: &Facility) -> Result<Facility> {
        let am = facility::Model {
            id: f.id,
            name: f.name.clone(),
            code: f.code.clone(),
            address: serde_json::to_value(&f.address).unwrap_or(serde_json::Value::Null),
            active: f.active,
            created_at: f.created_at.fixed_offset(),
            updated_at: f.updated_at.fixed_offset(),
        };
        am.insert(conn).await?;
        Ok(f.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Facility>> {
        let m = conn
            .one_as::<facility::Model>(Stmt::new(
                "SELECT * FROM facilities WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        Ok(m.map(facility_from_model))
    }

    pub async fn list<C: Conn>(conn: &C) -> Result<Vec<Facility>> {
        let rows = conn
            .all_as::<facility::Model>(Stmt::new(
                "SELECT * FROM facilities WHERE active = $1 ORDER BY name",
                [true.into()],
            ))
            .await?;
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
    pub async fn create<C: Conn>(conn: &C, w: &Ward) -> Result<Ward> {
        let am = ward::Model {
            id: w.id,
            facility_id: w.facility_id,
            name: w.name.clone(),
            code: w.code.clone(),
            capacity: w.capacity as i32,
            active: w.active,
            created_at: w.created_at.fixed_offset(),
            updated_at: w.updated_at.fixed_offset(),
        };
        am.insert(conn).await?;
        Ok(w.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Ward>> {
        let m = conn
            .one_as::<ward::Model>(Stmt::new(
                "SELECT * FROM wards WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        Ok(m.map(ward_from_model))
    }

    pub async fn list_by_facility<C: Conn>(conn: &C, facility_id: Uuid) -> Result<Vec<Ward>> {
        let rows = conn
            .all_as::<ward::Model>(Stmt::new(
                "SELECT * FROM wards WHERE facility_id = $1 AND active = $2 ORDER BY name",
                [facility_id.into(), true.into()],
            ))
            .await?;
        Ok(rows.into_iter().map(ward_from_model).collect())
    }
}

fn ward_from_model(m: ward::Model) -> Ward {
    Ward {
        id: m.id,
        facility_id: m.facility_id,
        name: m.name,
        code: m.code,
        capacity: Ord::max(m.capacity, 0) as u32,
        active: m.active,
        created_at: m.created_at.to_utc(),
        updated_at: m.updated_at.to_utc(),
    }
}

// ─── Room ───────────────────────────────────────────────────────────────

pub struct RoomRepository;

impl RoomRepository {
    pub async fn create<C: Conn>(conn: &C, r: &Room) -> Result<Room> {
        let am = room::Model {
            id: r.id,
            ward_id: r.ward_id,
            name: r.name.clone(),
            code: r.code.clone(),
            active: r.active,
            created_at: r.created_at.fixed_offset(),
            updated_at: r.updated_at.fixed_offset(),
        };
        am.insert(conn).await?;
        Ok(r.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Room>> {
        let m = conn
            .one_as::<room::Model>(Stmt::new(
                "SELECT * FROM rooms WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        Ok(m.map(room_from_model))
    }

    pub async fn list_by_ward<C: Conn>(conn: &C, ward_id: Uuid) -> Result<Vec<Room>> {
        let rows = conn
            .all_as::<room::Model>(Stmt::new(
                "SELECT * FROM rooms WHERE ward_id = $1 AND active = $2 ORDER BY name",
                [ward_id.into(), true.into()],
            ))
            .await?;
        Ok(rows.into_iter().map(room_from_model).collect())
    }

    /// Look up a room by its operator-assigned `code` (the second
    /// component of the HL7 v2 PL data type, e.g. `WARD^ROOM^BED`).
    /// Returns the first match, or `None` if the code isn't unique
    /// enough — callers wanting strict uniqueness should add a
    /// `(ward_id, code)` filter.
    pub async fn find_by_code<C: Conn>(conn: &C, code: &str) -> Result<Option<Room>> {
        let m = conn
            .one_as::<room::Model>(Stmt::new(
                "SELECT * FROM rooms WHERE code = $1 LIMIT 1",
                [code.into()],
            ))
            .await?;
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
