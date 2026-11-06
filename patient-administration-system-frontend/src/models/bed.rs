//! `beds` table — read-only.

use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct RoomRecord {
    id: Uuid,
    ward_id: Uuid,
    code: String,
}

#[derive(sqlx::FromRow)]
struct BedRecord {
    id: Uuid,
    room_id: Uuid,
    name: String,
    code: String,
    status: String,
}

#[derive(Debug, serde::Serialize)]
pub struct BedSummary {
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BedRow {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub status: String,
    pub room_code: String,
}

/// Walk rooms → beds for a ward and return the bed status strings only.
/// Used by the dashboard's ward-occupancy aggregate.
pub async fn list_by_ward(db: &PgPool, ward_id: Uuid) -> Result<Vec<BedSummary>, sqlx::Error> {
    let room_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM rooms WHERE ward_id = $1")
        .bind(ward_id)
        .fetch_all(db)
        .await?;
    if room_ids.is_empty() {
        return Ok(Vec::new());
    }
    let statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM beds WHERE room_id = ANY($1)")
            .bind(room_ids)
            .fetch_all(db)
            .await?;
    Ok(statuses
        .into_iter()
        .map(|status| BedSummary { status })
        .collect())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AvailableBed {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub room_code: String,
    pub ward_name: String,
    pub ward_code: String,
}

/// Every bed currently in `status = 'available'`, joined to its room +
/// ward for human-readable picker display. Sorted by ward + room + bed
/// code so adjacent beds group together visually.
pub async fn list_available(db: &PgPool) -> Result<Vec<AvailableBed>, sqlx::Error> {
    let beds: Vec<BedRecord> = sqlx::query_as(
        "SELECT id, room_id, name, code, status FROM beds WHERE status = 'available'",
    )
    .fetch_all(db)
    .await?;
    if beds.is_empty() {
        return Ok(Vec::new());
    }
    let room_ids: Vec<Uuid> = beds.iter().map(|b| b.room_id).collect();
    let rooms: Vec<RoomRecord> =
        sqlx::query_as("SELECT id, ward_id, code FROM rooms WHERE id = ANY($1)")
            .bind(room_ids)
            .fetch_all(db)
            .await?;
    let room_by_id: std::collections::HashMap<Uuid, RoomRecord> =
        rooms.into_iter().map(|r| (r.id, r)).collect();

    let ward_ids: Vec<Uuid> = room_by_id.values().map(|r| r.ward_id).collect();
    let wards = crate::models::ward::find_wards_by_ids(db, &ward_ids).await?;
    let ward_by_id: std::collections::HashMap<Uuid, crate::models::ward::WardSummary> =
        wards.into_iter().map(|w| (w.id, w)).collect();

    let mut out: Vec<AvailableBed> = beds
        .into_iter()
        .filter_map(|b| {
            let room = room_by_id.get(&b.room_id)?;
            let ward = ward_by_id.get(&room.ward_id)?;
            Some(AvailableBed {
                id: b.id,
                name: b.name,
                code: b.code,
                room_code: room.code.clone(),
                ward_name: ward.name.clone(),
                ward_code: ward.code.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a.ward_code
            .cmp(&b.ward_code)
            .then_with(|| a.room_code.cmp(&b.room_code))
            .then_with(|| a.code.cmp(&b.code))
    });
    Ok(out)
}

/// Rich projection used by the ward detail page: every bed in a ward
/// with its room code, sorted by `(room_code, bed_code)` for a stable
/// display order.
pub async fn list_by_ward_detailed(db: &PgPool, ward_id: Uuid) -> Result<Vec<BedRow>, sqlx::Error> {
    let rooms: Vec<RoomRecord> =
        sqlx::query_as("SELECT id, ward_id, code FROM rooms WHERE ward_id = $1")
            .bind(ward_id)
            .fetch_all(db)
            .await?;
    if rooms.is_empty() {
        return Ok(Vec::new());
    }
    let room_code_by_id: std::collections::HashMap<Uuid, String> =
        rooms.iter().map(|r| (r.id, r.code.clone())).collect();
    let room_ids: Vec<Uuid> = rooms.into_iter().map(|r| r.id).collect();
    let beds: Vec<BedRecord> =
        sqlx::query_as("SELECT id, room_id, name, code, status FROM beds WHERE room_id = ANY($1)")
            .bind(room_ids)
            .fetch_all(db)
            .await?;
    let mut rows: Vec<BedRow> = beds
        .into_iter()
        .map(|b| BedRow {
            id: b.id,
            name: b.name,
            code: b.code,
            status: b.status,
            room_code: room_code_by_id.get(&b.room_id).cloned().unwrap_or_default(),
        })
        .collect();
    rows.sort_by(|a, b| {
        a.room_code
            .cmp(&b.room_code)
            .then_with(|| a.code.cmp(&b.code))
    });
    Ok(rows)
}
