//! `schedules` + `slots` tables — read-only for the appointment booker.

use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ScheduleRow {
    pub id: Uuid,
    pub service_type: String,
}

pub async fn list_active_schedules(db: &PgPool) -> Result<Vec<ScheduleRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, service_type FROM schedules WHERE active = true ORDER BY service_type",
    )
    .fetch_all(db)
    .await
}

#[derive(sqlx::FromRow)]
struct SlotRecord {
    id: Uuid,
    schedule_id: Uuid,
    start_datetime: DateTime<FixedOffset>,
    end_datetime: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FreeSlotRow {
    pub id: Uuid,
    pub schedule_id: Uuid,
    /// ISO 8601 with no fractional seconds — easy to display + parse.
    pub start_iso: String,
    pub end_iso: String,
    /// Human-friendly label for the picker, e.g. `"Mon 2026-05-25 09:00 → 09:30"`.
    pub label: String,
}

/// Free slots for a given schedule starting between `now` and `now + days`.
pub async fn list_free_slots(
    db: &PgPool,
    schedule_id: Uuid,
    days: i64,
) -> Result<Vec<FreeSlotRow>, sqlx::Error> {
    let now: DateTime<FixedOffset> = Utc::now().fixed_offset();
    let horizon = now + Duration::days(days);
    let rows: Vec<SlotRecord> = sqlx::query_as(
        "SELECT id, schedule_id, start_datetime, end_datetime FROM slots \
         WHERE schedule_id = $1 AND status = 'free' \
           AND start_datetime >= $2 AND start_datetime <= $3 \
         ORDER BY start_datetime",
    )
    .bind(schedule_id)
    .bind(now)
    .bind(horizon)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|m| FreeSlotRow {
            id: m.id,
            schedule_id: m.schedule_id,
            label: format!(
                "{} {} → {}",
                m.start_datetime.format("%a %Y-%m-%d"),
                m.start_datetime.format("%H:%M"),
                m.end_datetime.format("%H:%M"),
            ),
            start_iso: m.start_datetime.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            end_iso: m.end_datetime.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        })
        .collect())
}
