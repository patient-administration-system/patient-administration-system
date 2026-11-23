//! The emergency department waits arithmetic: time to triage and to be seen against each category's target.

use super::targets::{Standing, Targets, standing};
use super::{CATEGORIES, colour};
use jiff::Timestamp;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CategoryWaits {
    pub category: &'static str,
    #[schema(required = true)]
    pub colour: Option<&'static str>,
    pub target_minutes: i64,
    pub attended: u32,
    pub seen_within: u32,
    pub seen_past: u32,
    pub waiting_within: u32,
    pub waiting_past: u32,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct PastTarget {
    pub encounter_id: Uuid,
    pub category: String,
    pub arrived_at: Timestamp,
    #[schema(required = true)]
    pub first_contact_at: Option<Timestamp>,
    /// Minutes from arrival to first contact, or to now when not yet seen.
    pub minutes: i64,
    pub target_minutes: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(as = triage::WaitsReport)]
pub struct WaitsReport {
    pub from: Timestamp,
    pub to: Timestamp,
    pub attendances: u32,
    /// Attendances with no triage category: left out of every figure below, not guessed.
    pub excluded_without_category: u32,
    pub by_category: Vec<CategoryWaits>,
    pub past_target: Vec<PastTarget>,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[schema(as = triage::QueueEntry)]
pub struct QueueEntry {
    pub encounter_id: Uuid,
    /// `None` when not yet triaged; those are listed last.
    #[schema(required = true)]
    pub category: Option<String>,
    #[schema(required = true)]
    pub colour: Option<&'static str>,
    pub arrived_at: Timestamp,
    pub waiting_minutes: i64,
    #[schema(required = true)]
    pub target_minutes: Option<i64>,
    pub past_target: bool,
}

/// One emergency attendance as read from the database.
pub struct Attendance {
    pub encounter_id: Uuid,
    pub arrived_at: Timestamp,
    pub first_contact_at: Option<Timestamp>,
    pub category: Option<String>,
}

pub fn minutes_between(a: Timestamp, b: Timestamp) -> i64 {
    b.duration_since(a).as_mins()
}

/// Pure: build the report from attendances.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "RCA-EXC-2: each counter counts attendances of one in-memory slice, so it is at most the slice's length, which is at most isize::MAX, below i64::MAX"
)]
pub fn build_report(
    attendances: &[Attendance],
    targets: &Targets,
    from: Timestamp,
    to: Timestamp,
    now: Timestamp,
) -> WaitsReport {
    let mut by_category: Vec<CategoryWaits> = CATEGORIES
        .iter()
        .map(|c| CategoryWaits {
            category: c,
            colour: colour(c),
            // RCA-DECL-1: every listed category has a target; a category without one has target 0, shown as none.
            target_minutes: targets.minutes(c).unwrap_or(0),
            attended: 0,
            seen_within: 0,
            seen_past: 0,
            waiting_within: 0,
            waiting_past: 0,
        })
        .collect();
    let mut past_target = Vec::new();
    let mut excluded = 0;
    for a in attendances {
        let Some(cat) = a.category.as_deref() else {
            excluded += 1;
            continue;
        };
        let Some(i) = CATEGORIES.iter().position(|c| *c == cat) else {
            excluded += 1;
            continue;
        };
        let Some(row) = by_category.get_mut(i) else {
            excluded += 1;
            continue;
        };
        let target = row.target_minutes;
        let s = standing(a.arrived_at, a.first_contact_at, now, target);
        row.attended += 1;
        match s {
            Standing::SeenWithin => row.seen_within += 1,
            Standing::SeenPast => row.seen_past += 1,
            Standing::WaitingWithin => row.waiting_within += 1,
            Standing::WaitingPast => row.waiting_past += 1,
        }
        if s.is_past() {
            past_target.push(PastTarget {
                encounter_id: a.encounter_id,
                category: cat.to_string(),
                arrived_at: a.arrived_at,
                first_contact_at: a.first_contact_at,
                minutes: minutes_between(a.arrived_at, a.first_contact_at.unwrap_or(now)),
                target_minutes: target,
            });
        }
    }
    past_target.sort_by_key(|p| (p.arrived_at, p.encounter_id));
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "RCA-CONV-3: the list holds the attendances of a window of at most 31 days, far below u32::MAX"
    )]
    let attended = attendances.len() as u32;
    WaitsReport {
        from,
        to,
        attendances: attended,
        excluded_without_category: excluded,
        by_category,
        past_target,
    }
}

/// Pure: the display order, most urgent category first then earliest arrival; not yet triaged last.
pub fn build_queue(
    attendances: &[Attendance],
    targets: &Targets,
    now: Timestamp,
) -> Vec<QueueEntry> {
    let mut entries: Vec<(usize, QueueEntry)> = attendances
        .iter()
        .filter(|a| a.first_contact_at.is_none())
        .map(|a| {
            let pos = a
                .category
                .as_deref()
                .and_then(|c| CATEGORIES.iter().position(|k| *k == c));
            let target = a.category.as_deref().and_then(|c| targets.minutes(c));
            let past = target.is_some_and(|t| standing(a.arrived_at, None, now, t).is_past());
            (
                pos.unwrap_or(CATEGORIES.len()),
                QueueEntry {
                    encounter_id: a.encounter_id,
                    category: pos.and(a.category.clone()),
                    colour: a.category.as_deref().and_then(colour),
                    arrived_at: a.arrived_at,
                    waiting_minutes: minutes_between(a.arrived_at, now),
                    target_minutes: target,
                    past_target: past,
                },
            )
        })
        .collect();
    entries.sort_by_key(|(pos, e)| (*pos, e.arrived_at, e.encounter_id));
    entries.into_iter().map(|(_, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::SignedDuration;

    fn t0() -> Timestamp {
        crate::time::utc(2026, 10, 23, 12, 0, 0)
    }

    fn att(cat: Option<&str>, arrive_min: i64, seen_min: Option<i64>) -> Attendance {
        Attendance {
            encounter_id: Uuid::new_v4(),
            arrived_at: t0() + SignedDuration::from_mins(arrive_min),
            first_contact_at: seen_min.map(|m| t0() + SignedDuration::from_mins(m)),
            category: cat.map(str::to_string),
        }
    }

    #[test]
    fn the_report_lands_nine_ten_and_eleven_minutes_on_the_right_side() {
        let rows = vec![
            att(Some("very_urgent"), 0, Some(9)),
            att(Some("very_urgent"), 0, Some(10)),
            att(Some("very_urgent"), 0, Some(11)),
        ];
        let r = build_report(
            &rows,
            &Targets::default(),
            t0(),
            t0() + crate::time::days(1),
            t0() + SignedDuration::from_hours(1),
        );
        let vu = &r.by_category[1];
        assert_eq!((vu.seen_within, vu.seen_past), (2, 1));
        assert_eq!(r.past_target.len(), 1);
        assert_eq!(r.past_target[0].minutes, 11);
    }

    #[test]
    fn an_attendance_with_no_category_is_excluded_not_guessed() {
        let rows = vec![att(None, 0, Some(500)), att(Some("standard"), 0, Some(5))];
        let r = build_report(
            &rows,
            &Targets::default(),
            t0(),
            t0() + crate::time::days(1),
            t0() + SignedDuration::from_hours(10),
        );
        assert_eq!((r.attendances, r.excluded_without_category), (2, 1));
        assert_eq!(r.by_category.iter().fold(0, |acc, c| acc + c.attended), 1);
        assert!(r.past_target.is_empty());
    }

    #[test]
    fn a_configured_target_changes_the_verdict() {
        let rows = vec![att(Some("urgent"), 0, Some(90))];
        let report = |t: &Targets| {
            build_report(
                &rows,
                t,
                t0(),
                t0() + crate::time::days(1),
                t0() + SignedDuration::from_hours(3),
            )
        };
        assert_eq!(report(&Targets::default()).past_target.len(), 1);
        let lenient = Targets::from_json(r#"{"urgent": 120}"#).unwrap();
        assert!(report(&lenient).past_target.is_empty());
    }

    #[test]
    fn the_queue_is_category_then_arrival_with_untriaged_last_and_seen_left_out() {
        let rows = vec![
            att(None, 0, None),
            att(Some("standard"), 1, None),
            att(Some("very_urgent"), 5, None),
            att(Some("very_urgent"), 2, None),
            att(Some("immediate"), 9, Some(10)),
        ];
        let q = build_queue(
            &rows,
            &Targets::default(),
            t0() + SignedDuration::from_mins(30),
        );
        let order: Vec<_> = q.iter().map(|e| e.category.as_deref()).collect();
        assert_eq!(
            order,
            vec![
                Some("very_urgent"),
                Some("very_urgent"),
                Some("standard"),
                None
            ]
        );
        assert!(q[0].arrived_at < q[1].arrived_at);
        assert!(q[0].past_target && !q[2].past_target);
        assert_eq!(q[3].target_minutes, None);
        assert!(!q[3].past_target);
    }
}
