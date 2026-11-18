//! TSC-1: one staff work queue. Needs PostgreSQL: `cargo test -- --ignored` (the last test reads source only).
//!
//! The queue keeps one work item for every open item of the existing lists and closes it when the source
//! closes. These tests make their own source rows (with fresh ids) and look only at the items for them, so
//! they hold on a database other runs have filled.

mod common;

use jiff::{SignedDuration, Timestamp};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Pool, Stmt};
use patient_administration_system::work_queue::{
    Filter, HIGH, NORMAL, WorkItem, WorkKind, WorkQueueService, WorkStatus,
};
use uuid::Uuid;

async fn db() -> Pool {
    common::build_state(&common::database_url().unwrap())
        .await
        .db
}

fn ctx(who: &str) -> UserContext {
    UserContext {
        user_id: Some(who.to_string()),
        user_ip: None,
        user_agent: None,
    }
}

/// The work item standing for `source` of `kind`, if any.
async fn item(
    db: &Pool,
    kind: WorkKind,
    source: Uuid,
) -> Option<(Uuid, String, i16, Option<String>)> {
    db.one(Stmt::new(
        "SELECT id, status, priority, owner FROM work_items WHERE kind = $1 AND source_id = $2",
        [kind.as_str().into(), source.into()],
    ))
    .await
    .unwrap()
    .map(|r| {
        (
            r.col("id").unwrap(),
            r.col("status").unwrap(),
            r.col("priority").unwrap(),
            r.col("owner").unwrap(),
        )
    })
}

async fn call_task(db: &Pool, reason: &str) -> Uuid {
    let id = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) VALUES ($1,$2,$3,$4,$5)",
        [
            id.into(),
            Uuid::new_v4().into(),
            Uuid::new_v4().into(),
            reason.into(),
            (Timestamp::now() + SignedDuration::from_hours(2)).into(),
        ],
    ))
    .await
    .unwrap();
    id
}

async fn handle_call(db: &Pool, id: Uuid) {
    db.run(Stmt::new(
        "UPDATE staff_call_tasks SET handled_at = now(), handled_by = 'Sam', call_outcome = 'reached' WHERE id = $1",
        [id.into()],
    ))
    .await
    .unwrap();
}

async fn profile_with_needs(db: &Pool) -> Uuid {
    let person = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO patient_communication_profiles (person_id, accessibility_needs) VALUES ($1, ARRAY['large_print'])",
        [person.into()],
    ))
    .await
    .unwrap();
    person
}

async fn alert(db: &Pool) -> Uuid {
    let id = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO tolerance_alerts (id, metric, scope_kind, scope_key, value, target, tolerance, escalate_beyond) \
         VALUES ($1, 'bed_occupancy', 'ward', $2, 0.99, 0.85, 0.05, 0.1)",
        [id.into(), format!("ward-{id}").into()],
    ))
    .await
    .unwrap();
    id
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn each_open_source_item_appears_once_and_closes_when_its_source_closes() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let silent = call_task(&db, "silent").await;
    let urgent = call_task(&db, "urgent_pathway").await;
    let person = profile_with_needs(&db).await;
    let a = alert(&db).await;

    svc.sync(Timestamp::now()).await.unwrap();
    // Once each, however often it runs.
    svc.sync(Timestamp::now()).await.unwrap();
    let n: i64 = db
        .one(Stmt::new(
            "SELECT count(*)::bigint AS n FROM work_items WHERE source_id = ANY($1)",
            [vec![silent, urgent, person, a].into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .col("n")
        .unwrap();
    assert_eq!(n, 4);
    assert_eq!(item(&db, WorkKind::Call, silent).await.unwrap().2, NORMAL);
    assert_eq!(
        item(&db, WorkKind::Call, urgent).await.unwrap().2,
        HIGH,
        "an urgent pathway is high"
    );
    assert_eq!(
        item(&db, WorkKind::NeedsReview, person).await.unwrap().1,
        "open"
    );
    assert_eq!(
        item(&db, WorkKind::ToleranceAlert, a).await.unwrap().2,
        HIGH
    );

    // The call is made, in its own place: its item closes and keeps no claim.
    handle_call(&db, silent).await;
    db.run(Stmt::new(
        "UPDATE tolerance_alerts SET acknowledged_by = 'Sam', acknowledged_at = now() WHERE id = $1",
        [a.into()],
    ))
    .await
    .unwrap();
    let r = svc.sync(Timestamp::now()).await.unwrap();
    assert!(r.closed >= 2, "{r:?}");
    assert_eq!(item(&db, WorkKind::Call, silent).await.unwrap().1, "closed");
    assert_eq!(
        item(&db, WorkKind::ToleranceAlert, a).await.unwrap().1,
        "closed"
    );
    assert_eq!(
        item(&db, WorkKind::Call, urgent).await.unwrap().1,
        "open",
        "the other call is still open"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_closed_item_opens_again_when_its_source_does() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let person = profile_with_needs(&db).await;
    svc.sync(Timestamp::now()).await.unwrap();
    // Reviewed: closed.
    db.run(Stmt::new(
        "UPDATE patient_communication_profiles SET needs_reviewed_at = now(), needs_reviewed_by = 'Sam' WHERE person_id = $1",
        [person.into()],
    ))
    .await
    .unwrap();
    svc.sync(Timestamp::now()).await.unwrap();
    assert_eq!(
        item(&db, WorkKind::NeedsReview, person).await.unwrap().1,
        "closed"
    );
    // The review lapses (here: the record is cleared): open again, the same item, nobody's.
    db.run(Stmt::new(
        "UPDATE patient_communication_profiles SET needs_reviewed_at = NULL, needs_reviewed_by = NULL WHERE person_id = $1",
        [person.into()],
    ))
    .await
    .unwrap();
    let r = svc.sync(Timestamp::now()).await.unwrap();
    assert!(r.reopened >= 1, "{r:?}");
    let (_, status, _, owner) = item(&db, WorkKind::NeedsReview, person).await.unwrap();
    assert_eq!((status.as_str(), owner), ("open", None));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn claim_release_and_assign_are_owned_and_audited() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let task = call_task(&db, "silent").await;
    svc.sync(Timestamp::now()).await.unwrap();
    let (id, ..) = item(&db, WorkKind::Call, task).await.unwrap();
    let sam = format!("sam-{}", Uuid::new_v4());
    let kim = format!("kim-{}", Uuid::new_v4());

    let claimed = svc.claim(id, &sam, &ctx(&sam)).await.unwrap();
    assert_eq!(
        (claimed.status, claimed.owner.as_deref()),
        (WorkStatus::Claimed, Some(sam.as_str()))
    );
    // Claiming again is no change; someone else claiming is a conflict.
    assert_eq!(
        svc.claim(id, &sam, &ctx(&sam)).await.unwrap().owner,
        claimed.owner
    );
    assert!(svc.claim(id, &kim, &ctx(&kim)).await.is_err());
    // Only the owner can release.
    assert!(svc.release(id, &kim, &ctx(&kim)).await.is_err());
    // "Mine" lists it.
    let mine = svc
        .list(
            &Filter {
                owner: Some(sam.clone()),
                ..Filter::default()
            },
            50,
        )
        .await
        .unwrap();
    assert!(mine.iter().any(|w| w.id == id));
    let released = svc.release(id, &sam, &ctx(&sam)).await.unwrap();
    assert_eq!((released.status, released.owner), (WorkStatus::Open, None));
    // Reassigned to a named person; a blank name is refused.
    assert!(svc.assign(id, "  ", &ctx(&sam)).await.is_err());
    assert_eq!(
        svc.assign(id, &kim, &ctx(&sam))
            .await
            .unwrap()
            .owner
            .as_deref(),
        Some(kim.as_str())
    );
    // Every change is in the audit log, with who did it.
    let actions: Vec<String> = db
        .all(Stmt::new(
            "SELECT action FROM audit_log WHERE entity_type = 'work_item' AND entity_id = $1 ORDER BY at",
            [id.into()],
        ))
        .await
        .unwrap()
        .iter()
        .map(|r| r.col("action").unwrap())
        .collect();
    assert_eq!(actions, ["claim", "release", "assign"]);
    // Closing the source closes the item; a closed item cannot be claimed.
    handle_call(&db, task).await;
    svc.sync(Timestamp::now()).await.unwrap();
    assert!(svc.claim(id, &sam, &ctx(&sam)).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn taking_the_next_item_gives_two_people_two_different_items() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let a = alert(&db).await;
    let b = alert(&db).await;
    svc.sync(Timestamp::now()).await.unwrap();
    let one = format!("one-{}", Uuid::new_v4());
    let two = format!("two-{}", Uuid::new_v4());
    let (c1, c2) = (ctx(&one), ctx(&two));
    let (x, y) = tokio::join!(
        svc.take_next("operations", &one, &c1),
        svc.take_next("operations", &two, &c2),
    );
    let (x, y): (WorkItem, WorkItem) = (x.unwrap().unwrap(), y.unwrap().unwrap());
    assert_ne!(x.id, y.id, "never the same item to two people");
    assert_eq!(x.owner.as_deref(), Some(one.as_str()));
    assert_eq!(y.owner.as_deref(), Some(two.as_str()));
    assert!(
        svc.take_next("nobody", &one, &ctx(&one)).await.is_err(),
        "an unknown queue is refused"
    );
    // Tidy: close the two alerts so the next run's operations queue is not longer for them.
    for id in [a, b] {
        db.run(Stmt::new(
            "UPDATE tolerance_alerts SET cleared_at = now() WHERE id = $1",
            [id.into()],
        ))
        .await
        .unwrap();
    }
    svc.sync(Timestamp::now()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_overdue_view_lists_only_items_due_before_now() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let late = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) VALUES ($1,$2,$3,'silent',$4)",
        [
            late.into(),
            Uuid::new_v4().into(),
            Uuid::new_v4().into(),
            (Timestamp::now() - SignedDuration::from_hours(3)).into(),
        ],
    ))
    .await
    .unwrap();
    let soon = call_task(&db, "silent").await;
    svc.sync(Timestamp::now()).await.unwrap();
    let (late_id, ..) = item(&db, WorkKind::Call, late).await.unwrap();
    let (soon_id, ..) = item(&db, WorkKind::Call, soon).await.unwrap();
    // Given to one person, so the view is of these two whatever else is overdue on this database.
    let who = format!("pat-{}", Uuid::new_v4());
    for id in [late_id, soon_id] {
        svc.assign(id, &who, &ctx(&who)).await.unwrap();
    }
    let overdue = svc
        .list(
            &Filter {
                queue: Some("booking".into()),
                owner: Some(who.clone()),
                overdue: true,
                ..Filter::default()
            },
            50,
        )
        .await
        .unwrap();
    assert!(
        overdue
            .iter()
            .all(|w| w.overdue && w.due_at < Timestamp::now())
    );
    assert!(overdue.iter().any(|w| w.id == late_id));
    assert!(!overdue.iter().any(|w| w.id == soon_id));
    for id in [late, soon] {
        handle_call(&db, id).await;
    }
    svc.sync(Timestamp::now()).await.unwrap();
}

/// NFR-11: the queue asks a person to act and never acts itself. Its source names no write to the records that
/// decide a patient's care or to the lists it reads.
#[test]
fn the_work_queue_never_writes_to_a_patients_records_or_to_its_sources() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/work_queue/mod.rs"
    ))
    .unwrap();
    let code = src.split("#[cfg(test)]").next().unwrap().to_lowercase();
    for table in [
        "appointments",
        "referrals",
        "rtt_pathways",
        "waitlist_entries",
        "slots",
        "staff_call_tasks",
        "follow_up_tasks",
        "validation_items",
        "patient_communication_profiles",
        "tolerance_alerts",
    ] {
        for verb in ["update ", "insert into ", "delete from "] {
            let needle = format!("{verb}{table}");
            assert!(!code.contains(&needle), "the work queue must not {needle}");
        }
    }
}

/// A call task due at `due`.
async fn call_due(db: &Pool, due: Timestamp) -> Uuid {
    let id = Uuid::new_v4();
    db.run(Stmt::new(
        "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) VALUES ($1,$2,$3,'silent',$4)",
        [id.into(), Uuid::new_v4().into(), Uuid::new_v4().into(), due.into()],
    ))
    .await
    .unwrap();
    id
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn reminders_are_the_owners_items_past_due_or_due_by_the_end_of_the_working_day() {
    let db = db().await;
    let svc = WorkQueueService::new(db.clone());
    let now = Timestamp::now();
    let late = call_due(&db, now - SignedDuration::from_hours(2)).await;
    let soon = call_due(&db, now + SignedDuration::from_mins(1)).await;
    let later = call_due(&db, now + patient_administration_system::time::days(30)).await;
    svc.sync(now).await.unwrap();
    let who = format!("rem-{}", Uuid::new_v4());
    for t in [late, soon, later] {
        let (id, ..) = item(&db, WorkKind::Call, t).await.unwrap();
        svc.assign(id, &who, &ctx(&who)).await.unwrap();
    }
    let r = svc.reminders(&who, Timestamp::now()).await.unwrap();
    let ids = |v: &[WorkItem]| v.iter().map(|w| w.source_id).collect::<Vec<_>>();
    assert_eq!(ids(&r.overdue), [late], "past due");
    assert_eq!(
        ids(&r.due_today),
        [soon],
        "due before the end of the working day"
    );
    assert!(r.due_by > Timestamp::now());
    // Someone else's reminders are not these.
    assert!(
        svc.reminders("someone-else", Timestamp::now())
            .await
            .unwrap()
            .overdue
            .iter()
            .all(|w| w.owner.as_deref() != Some(who.as_str()))
    );
    for t in [late, soon, later] {
        handle_call(&db, t).await;
    }
    svc.sync(Timestamp::now()).await.unwrap();
}
