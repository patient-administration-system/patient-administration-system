//! NEW-8: clinic session templates, slot types, ring-fencing and the capacity report. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Utc};
use patient_administration_system::clinic_templates::report::capacity_report;
use patient_administration_system::clinic_templates::{
    Generated, Line, TemplateService, VersionInput,
};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::referrals::NewReferral;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

fn lines() -> Vec<Line> {
    vec![
        Line {
            slot_type: "new".into(),
            slots: 2,
            ring_fenced: false,
        },
        Line {
            slot_type: "follow_up".into(),
            slots: 1,
            ring_fenced: false,
        },
        Line {
            slot_type: "urgent".into(),
            slots: 1,
            ring_fenced: true,
        },
    ]
}

fn input(day: NaiveDate, by: &str) -> VersionInput {
    VersionInput {
        weekday: day.weekday().number_from_monday() as i16,
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        slot_minutes: 20,
        effective_from: day,
        lines: lines(),
        proposed_by: by.into(),
    }
}

/// A date about two weeks out, so it is neither past nor close to a clock change in a way that matters.
fn day() -> NaiveDate {
    (Utc::now() + Duration::days(14)).date_naive()
}

async fn slots_of(db: &sea_orm::DatabaseConnection, template: Uuid) -> Vec<(String, String, bool)> {
    db.query_all_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT to_char(start_datetime AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') AS at, slot_type, ring_fenced \
         FROM slots WHERE template_id = $1 ORDER BY start_datetime",
        [template.into()],
    ))
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.try_get("", "at").unwrap(),
            r.try_get("", "slot_type").unwrap(),
            r.try_get("", "ring_fenced").unwrap(),
        )
    })
    .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_version_makes_slots_only_once_a_different_person_approves_it_and_generating_is_reproducible()
 {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = TemplateService::new(state.db.clone());
    let ctx = UserContext::system();
    let now = Utc::now();
    let schedule = common::new_schedule(&state.db).await.id;
    let t = svc
        .create(schedule, "Tuesday clinic", "Sister Hughes", &ctx)
        .await
        .unwrap();
    let d = day();
    let v = svc
        .propose(t, &input(d, "Sister Hughes"), now, &ctx)
        .await
        .unwrap();
    assert_eq!(v, 1);

    // Unapproved: nothing is made.
    let g = svc
        .generate(t, d, d, "Sister Hughes", now, &ctx)
        .await
        .unwrap();
    assert_eq!(
        g,
        Generated {
            days_without_approved_version: 1,
            ..Default::default()
        }
    );

    // The proposer cannot approve their own version, however they spell their name.
    assert!(svc.approve(t, v, "Sister Hughes", now, &ctx).await.is_err());
    assert!(
        svc.approve(t, v, "  sister HUGHES ", now, &ctx)
            .await
            .is_err()
    );
    assert!(svc.approve(t, v, "   ", now, &ctx).await.is_err());
    svc.approve(t, v, "Dr Evans", now, &ctx).await.unwrap();
    assert!(
        svc.approve(t, v, "Dr Evans", now, &ctx).await.is_err(),
        "once only"
    );
    assert_eq!(
        common::audit_count(&state.db, t, "approve_version").await,
        1
    );

    // A fortnight holds this weekday twice: two sessions of four slots.
    let to = d + Duration::days(13);
    let g = svc.generate(t, d, to, "Dr Evans", now, &ctx).await.unwrap();
    assert_eq!(g.created, 8);
    assert_eq!(g.already_there, 0);
    let made = slots_of(&state.db, t).await;
    assert_eq!(made.len(), 8);
    let types: Vec<_> = made[..4].iter().map(|s| (s.1.as_str(), s.2)).collect();
    assert_eq!(
        types,
        [
            ("new", false),
            ("new", false),
            ("follow_up", false),
            ("urgent", true)
        ]
    );

    // Doing it again adds nothing and changes nothing.
    let again = svc.generate(t, d, to, "Dr Evans", now, &ctx).await.unwrap();
    assert_eq!((again.created, again.already_there), (0, 8));
    assert_eq!(slots_of(&state.db, t).await, made);

    // Two schedules with the same approved version make the same pattern of slots.
    let schedule2 = common::new_schedule(&state.db).await.id;
    let t2 = svc
        .create(schedule2, "Same again", "Sister Hughes", &ctx)
        .await
        .unwrap();
    let v2 = svc
        .propose(t2, &input(d, "Sister Hughes"), now, &ctx)
        .await
        .unwrap();
    svc.approve(t2, v2, "Dr Evans", now, &ctx).await.unwrap();
    svc.generate(t2, d, to, "Dr Evans", now, &ctx)
        .await
        .unwrap();
    assert_eq!(slots_of(&state.db, t2).await, made);

    // Invalid proposals and ranges are refused.
    let mut bad = input(d, "Sister Hughes");
    bad.lines = vec![Line {
        slot_type: "routine".into(),
        slots: 1,
        ring_fenced: false,
    }];
    assert!(svc.propose(t, &bad, now, &ctx).await.is_err());
    let mut past = input(d, "Sister Hughes");
    past.effective_from = (now - Duration::days(3)).date_naive();
    assert!(svc.propose(t, &past, now, &ctx).await.is_err());
    assert!(
        svc.generate(t, d, d + Duration::days(92), "Dr Evans", now, &ctx)
            .await
            .is_err()
    );
    assert!(
        svc.generate(t, d, d - Duration::days(1), "Dr Evans", now, &ctx)
            .await
            .is_err()
    );
    assert!(svc.generate(t, d, d, " ", now, &ctx).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_newer_version_applies_from_its_date_and_leaves_earlier_slots_alone() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = TemplateService::new(state.db.clone());
    let ctx = UserContext::system();
    let now = Utc::now();
    let schedule = common::new_schedule(&state.db).await.id;
    let t = svc
        .create(schedule, "Changing clinic", "Sister Hughes", &ctx)
        .await
        .unwrap();
    let d = day();
    let v1 = svc
        .propose(t, &input(d, "Sister Hughes"), now, &ctx)
        .await
        .unwrap();
    svc.approve(t, v1, "Dr Evans", now, &ctx).await.unwrap();
    // From the second occurrence the session is longer slots and a single line.
    let mut second = input(d + Duration::days(7), "Dr Evans");
    second.slot_minutes = 30;
    second.lines = vec![Line {
        slot_type: "new".into(),
        slots: 3,
        ring_fenced: false,
    }];
    let v2 = svc.propose(t, &second, now, &ctx).await.unwrap();
    assert_eq!(v2, 2);
    // Proposed but unapproved: the first version still governs, and only the first day is made.
    let g = svc.generate(t, d, d, "Dr Evans", now, &ctx).await.unwrap();
    assert_eq!(g.created, 4);
    // Approved: the day already made is untouched; the next two days use version 2 (three slots each).
    svc.approve(t, v2, "Sister Hughes", now, &ctx)
        .await
        .unwrap();
    let g = svc
        .generate(t, d, d + Duration::days(14), "Dr Evans", now, &ctx)
        .await
        .unwrap();
    assert_eq!(g.already_there, 4, "the first day is already there");
    assert_eq!(g.created, 6, "the next two days use version 2");
    let used: Vec<i32> = state
        .db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT DISTINCT template_version AS v FROM slots WHERE template_id = $1 ORDER BY 1",
            [t.into()],
        ))
        .await
        .unwrap()
        .iter()
        .map(|r| r.try_get("", "v").unwrap())
        .collect();
    assert_eq!(used, [1, 2]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_ring_fenced_slot_is_not_offered_automatically_but_can_be_booked_by_choice() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = TemplateService::new(state.db.clone());
    let ctx = UserContext::system();
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&state.db, &service).await.id;
    let fenced = common::free_slot(&state.db, schedule, 3).await;
    let open = common::free_slot(&state.db, schedule, 5).await;
    svc.set_ring_fence(fenced.id, true, "Sister Hughes", &ctx)
        .await
        .unwrap();
    assert!(
        svc.set_ring_fence(fenced.id, true, " ", &ctx)
            .await
            .is_err()
    );
    assert_eq!(
        common::audit_count(&state.db, fenced.id, "set_ring_fence").await,
        1
    );

    // The referral's first available slot skips the earlier, ring-fenced one.
    let (r, _) = state
        .referrals
        .create(
            NewReferral {
                person_id: Uuid::new_v4(),
                source: "e-rs".into(),
                external_ref: Some(format!("U{}", Uuid::new_v4().simple())),
                target_service: service.clone(),
                reason: None,
                referring_worker_id: None,
                received_at: None,
            },
            &ctx,
        )
        .await
        .unwrap();
    let booked = state
        .referrals
        .book_first_available(r.id, &ctx)
        .await
        .unwrap();
    assert_eq!(booked.appointment.slot_id, Some(open.id));

    // A person who picks the ring-fenced slot can still book it.
    let appt = state
        .scheduling
        .book_slot(fenced.id, Uuid::new_v4(), &ctx)
        .await
        .unwrap();
    assert_eq!(appt.slot_id, Some(fenced.id));
    // A booked slot cannot be ring-fenced or released.
    assert!(
        svc.set_ring_fence(fenced.id, false, "Sister Hughes", &ctx)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_capacity_report_matches_a_worked_example() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let ctx = UserContext::system();
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&state.db, &service).await.id;
    let from = Utc::now();
    let to = from + Duration::days(28); // four weeks forward

    // Four weeks back: 8 referrals, so 2 a week.
    for i in 0..8 {
        state
            .referrals
            .create(
                NewReferral {
                    person_id: Uuid::new_v4(),
                    source: "e-rs".into(),
                    external_ref: Some(format!("U{}", Uuid::new_v4().simple())),
                    target_service: service.clone(),
                    reason: None,
                    referring_worker_id: None,
                    received_at: Some(from - Duration::days(2 + i * 3)),
                },
                &ctx,
            )
            .await
            .unwrap();
    }
    // Of 10 past appointments, 9 attended and 1 missed: a rate of 0.1.
    for i in 0..10 {
        let start = from - Duration::days(3 + i);
        let slot = Uuid::new_v4();
        state
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO slots (id, schedule_id, start_datetime, end_datetime, status, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, 'busy', now(), now())",
                [slot.into(), schedule.into(), start.into(), (start + Duration::minutes(20)).into()],
            ))
            .await
            .unwrap();
        state
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO appointments (id, person_id, slot_id, start_datetime, end_datetime, status, \
                   created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, now(), now())",
                [
                    Uuid::new_v4().into(),
                    Uuid::new_v4().into(),
                    slot.into(),
                    start.into(),
                    (start + Duration::minutes(20)).into(),
                    (if i == 0 { "no_show" } else { "fulfilled" }).into(),
                ],
            ))
            .await
            .unwrap();
    }
    // Forward: 40 slots in four weeks (10 a week), 4 of them ring-fenced.
    for i in 0..40 {
        let start = from + Duration::days(1) + Duration::hours(i);
        state
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO slots (id, schedule_id, start_datetime, end_datetime, status, slot_type, \
                   ring_fenced, created_at, updated_at) VALUES ($1, $2, $3, $4, 'free', $5, $6, now(), now())",
                [
                    Uuid::new_v4().into(),
                    schedule.into(),
                    start.into(),
                    (start + Duration::minutes(30)).into(),
                    (if i < 4 { "urgent" } else { "new" }).into(),
                    (i < 4).into(),
                ],
            ))
            .await
            .unwrap();
    }

    let r = capacity_report(&state.db, &service, from, to, 28)
        .await
        .unwrap();
    assert_eq!(r.referrals_in_lookback, 8);
    assert_eq!(r.demand_per_week, 2.0);
    assert_eq!((r.attended, r.missed), (9, 1));
    assert_eq!(r.non_attendance_rate, Some(0.1));
    // 2 / (1 - 0.1) = 2.2 slots a week needed; 10 offered: 7.8 to spare.
    assert_eq!(r.required_per_week, 2.2);
    assert_eq!(r.available_slots, 40);
    assert_eq!(r.available_per_week, 10.0);
    assert_eq!(r.ring_fenced_slots, 4);
    assert_eq!(r.gap_per_week, 7.8);
    let urgent = r.by_type.iter().find(|t| t.slot_type == "urgent").unwrap();
    assert_eq!((urgent.slots, urgent.ring_fenced), (4, 4));
    assert!(r.method.contains("not NHS England's core model"));

    // A backwards window and a missing service are refused.
    assert!(
        capacity_report(&state.db, &service, to, from, 28)
            .await
            .is_err()
    );
    assert!(capacity_report(&state.db, " ", from, to, 28).await.is_err());
}
