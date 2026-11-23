//! RTT (Referral-To-Treatment) pathways — read-side projection for the
//! `/rtt` cockpit.
//!
//! Loads every non-stopped pathway, walks its `rtt_clock_events` to compute
//! weeks-waiting, and joins to the patient's name. The weeks-waiting
//! formula mirrors `crate::models::rtt::compute_active_weeks` on the PAS
//! Axum side — sum of unpaused intervals from `started`/`resumed` events
//! to either the next `paused`/`stopped` event or `now`.

use crate::models::patient::names_for;
use jiff::Timestamp;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct PathwayRecord {
    id: Uuid,
    person_id: Uuid,
    target_service: String,
    breach_weeks: i32,
    status: String,
}

#[derive(sqlx::FromRow)]
struct EventRecord {
    kind: String,
    event_at: jiff_sqlx::Timestamp,
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
#[expect(
    clippy::arithmetic_side_effects,
    reason = "RCA-EXC-2: the sort compares the difference of two u32 values widened to i64, which always fits"
)]
pub async fn list_active_pathways(db: &PgPool) -> Result<Vec<PathwayRow>, sqlx::Error> {
    let pathways: Vec<PathwayRecord> = sqlx::query_as(
        "SELECT id, person_id, target_service, breach_weeks, status FROM rtt_pathways \
         WHERE status <> 'stopped'",
    )
    .fetch_all(db)
    .await?;
    if pathways.is_empty() {
        return Ok(Vec::new());
    }

    // Patient name lookup table.
    let person_ids: Vec<Uuid> = pathways.iter().map(|p| p.person_id).collect();
    let names = names_for(&person_ids).await;

    let now = Timestamp::now();
    let mut rows = Vec::with_capacity(pathways.len());
    for p in pathways {
        // Pull events for this pathway in chronological order.
        let events: Vec<EventRecord> = sqlx::query_as(
            "SELECT kind, event_at FROM rtt_clock_events WHERE pathway_id = $1 ORDER BY event_at",
        )
        .bind(p.id)
        .fetch_all(db)
        .await?;
        let weeks = compute_active_weeks_from_models(&events, now);
        // RCA-DECL-1: a negative threshold is read as none; `max(0)` leaves no other failure.
        let breach_weeks = u32::try_from(p.breach_weeks.max(0)).unwrap_or(0);
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
#[expect(
    clippy::arithmetic_side_effects,
    reason = "RCA-EXC-2: each term is the span between two timestamps, under 6.4e11 seconds, and there is one per event already held in memory, so the sum stays far below i64::MAX"
)]
fn compute_active_weeks_from_models(events: &[EventRecord], now: Timestamp) -> u32 {
    if events.is_empty() {
        return 0;
    }
    let mut total_seconds: i64 = 0;
    let mut active_since: Option<Timestamp> = None;
    for e in events {
        match e.kind.as_str() {
            "started" | "resumed" => active_since = Some(e.event_at.to_jiff()),
            "paused" | "stopped" => {
                if let Some(start) = active_since.take() {
                    total_seconds += e.event_at.to_jiff().duration_since(start).as_secs();
                }
            }
            _ => {} // unknown kind — ignore
        }
    }
    if let Some(start) = active_since {
        total_seconds += now.duration_since(start).as_secs();
    }
    const SECONDS_PER_WEEK: i64 = 7 * 24 * 3600;
    if total_seconds < 0 {
        return 0;
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "RCA-CONV-3: the events are in chronological order, so the spans are disjoint and lie inside the calendar's 3.2e11 seconds, which is under 5.3e5 weeks"
    )]
    let weeks = (total_seconds / SECONDS_PER_WEEK) as u32;
    weeks
}
