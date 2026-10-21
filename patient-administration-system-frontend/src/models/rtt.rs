//! RTT (Referral-To-Treatment) pathways — read-side projection for the
//! `/rtt` cockpit.
//!
//! Loads every non-stopped pathway, walks its `rtt_clock_events` to compute
//! weeks-waiting, and joins to the patient's name. The weeks-waiting
//! formula mirrors `crate::models::rtt::compute_active_weeks` on the PAS
//! Axum side — sum of unpaused intervals from `started`/`resumed` events
//! to either the next `paused`/`stopped` event or `now`.

use crate::models::patient::names_for;
use chrono::{DateTime, FixedOffset, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use uuid::Uuid;

mod pathway_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "rtt_pathways")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub person_id: Uuid,
        pub target_service: String,
        pub breach_weeks: i32,
        pub status: String,
        pub started_at: chrono::DateTime<chrono::FixedOffset>,
        pub stopped_at: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod event_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "rtt_clock_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub pathway_id: Uuid,
        pub kind: String,
        pub reason: Option<String>,
        pub event_at: chrono::DateTime<chrono::FixedOffset>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// One row in the `/rtt` cockpit.
#[derive(Debug, Clone, Serialize)]
pub struct PathwayRow {
    pub pathway_id: Uuid,
    pub person_id: Uuid,
    pub patient_family: String,
    pub patient_given: String,
    pub target_service: String,
    pub status: String,
    pub weeks_waiting: u32,
    pub breach_weeks: u32,
    pub is_breaching: bool,
}

/// Load every non-stopped pathway, compute weeks-waiting per pathway, and
/// return rows sorted worst-breach-first. The page renders the result
/// straight to a Lily `data-table`.
pub async fn list_active_pathways(db: &DatabaseConnection) -> Result<Vec<PathwayRow>, DbErr> {
    let pathways = pathway_entity::Entity::find()
        .filter(pathway_entity::Column::Status.ne("stopped"))
        .all(db)
        .await?;
    if pathways.is_empty() {
        return Ok(Vec::new());
    }

    // Patient name lookup table.
    let person_ids: Vec<Uuid> = pathways.iter().map(|p| p.person_id).collect();
    let names = names_for(&person_ids).await;

    let now = Utc::now();
    let mut rows = Vec::with_capacity(pathways.len());
    for p in pathways {
        // Pull events for this pathway in chronological order.
        let events = event_entity::Entity::find()
            .filter(event_entity::Column::PathwayId.eq(p.id))
            .order_by_asc(event_entity::Column::EventAt)
            .all(db)
            .await?;
        let weeks = compute_active_weeks_from_models(&events, now);
        let breach_weeks = p.breach_weeks.max(0) as u32;
        let (family, given) = names
            .get(&p.person_id)
            .cloned()
            .unwrap_or_else(|| ("(unknown)".into(), String::new()));
        rows.push(PathwayRow {
            pathway_id: p.id,
            person_id: p.person_id,
            patient_family: family,
            patient_given: given,
            target_service: p.target_service,
            status: p.status,
            weeks_waiting: weeks,
            breach_weeks,
            is_breaching: weeks > breach_weeks,
        });
    }

    // Worst breaches first (largest excess over threshold), then by raw
    // weeks waiting, then alphabetical on service.
    rows.sort_by(|a, b| {
        let a_excess = i64::from(a.weeks_waiting) - i64::from(a.breach_weeks);
        let b_excess = i64::from(b.weeks_waiting) - i64::from(b.breach_weeks);
        b_excess
            .cmp(&a_excess)
            .then_with(|| b.weeks_waiting.cmp(&a.weeks_waiting))
            .then_with(|| a.target_service.cmp(&b.target_service))
    });
    Ok(rows)
}

/// Sum unpaused intervals from `started`/`resumed` events to either the
/// next `paused`/`stopped` event or `now`, then floor-divide by a week.
/// Mirrors `compute_active_weeks` from the PAS Axum lib.
fn compute_active_weeks_from_models(events: &[event_entity::Model], now: DateTime<Utc>) -> u32 {
    if events.is_empty() {
        return 0;
    }
    let mut total_seconds: i64 = 0;
    let mut active_since: Option<DateTime<FixedOffset>> = None;
    for e in events {
        match e.kind.as_str() {
            "started" | "resumed" => active_since = Some(e.event_at),
            "paused" | "stopped" => {
                if let Some(start) = active_since.take() {
                    total_seconds += (e.event_at - start).num_seconds();
                }
            }
            _ => {} // unknown kind — ignore
        }
    }
    if let Some(start) = active_since {
        let now_fixed: DateTime<FixedOffset> =
            now.with_timezone(&FixedOffset::east_opt(0).unwrap());
        total_seconds += (now_fixed - start).num_seconds();
    }
    const SECONDS_PER_WEEK: i64 = 7 * 24 * 3600;
    if total_seconds < 0 {
        return 0;
    }
    (total_seconds / SECONDS_PER_WEEK) as u32
}
