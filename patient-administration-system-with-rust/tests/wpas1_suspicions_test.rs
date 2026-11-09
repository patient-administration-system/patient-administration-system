//! WPAS-1 and WPAS-8 (spec 20, FR-207 to FR-221): tracking suspicion during referrals, under a configurable
//! profile. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test builds the service with the profile it means (`wales` for the cancer rules, `generic` for the
//! default), so nothing depends on the environment. Other runs leave suspicions behind, so the list and the
//! report are checked with rows the test can find by id, or by differences.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, NaiveDate, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::waitlist::Priority;
use patient_administration_system::referrals::NewReferral;
use patient_administration_system::suspicions::{
    NewSuspicion, Profile, StopReason, SuspicionService, TargetStatus, today,
};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

fn service(s: &AppState, profile: &str) -> SuspicionService {
    SuspicionService::new(s.db.clone(), Arc::new(Profile::load(profile).unwrap()))
}

/// A referral of a new person to `service`.
async fn referral(s: &AppState, service: &str) -> Uuid {
    let (r, _) = s
        .referrals
        .create(
            NewReferral {
                person_id: Uuid::new_v4(),
                source: "gp".into(),
                external_ref: None,
                target_service: service.into(),
                reason: None,
                referring_worker_id: None,
                received_at: None,
            },
            &sys(),
        )
        .await
        .unwrap();
    r.id
}

fn new(condition: &str, subtype: Option<&str>, days_ago: i64) -> NewSuspicion {
    NewSuspicion {
        condition: condition.into(),
        subtype: subtype.map(String::from),
        unique_pathway_identifier: None,
        suspected_on: today() - Duration::days(days_ago),
        suspected_by: "Dr Rhian Evans".into(),
        recorded_by: "Gareth Jones".into(),
    }
}

/// The referral's waiting-list entry: its id and priority.
async fn entry_of(s: &AppState, referral: Uuid) -> (Uuid, String) {
    let r =
        s.db.one(Stmt::new(
            "SELECT id, priority FROM waitlist_entries WHERE referral_id = $1",
            [referral.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    (r.col("id").unwrap(), r.col("priority").unwrap())
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    if_match: Option<&str>,
    body: serde_json::Value,
) -> (StatusCode, Option<String>, serde_json::Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = if_match {
        b = b.header("if-match", t);
    }
    let r = app
        .clone()
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let st = r.status();
    let tag = r
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let v = serde_json::from_slice(&to_bytes(r.into_body(), 1 << 20).await.unwrap())
        .unwrap_or_default();
    (st, tag, v)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn records_a_suspicion_and_refuses_bad_ones() {
    let s = state().await;
    let wales = service(&s, "wales");
    let r = referral(&s, "wpas1-record").await;

    // Under the profile with a required site list, cancer needs a site from it.
    let mut n = new("cancer", Some("04"), 3);
    n.unique_pathway_identifier = Some("UPI-0001".into());
    let one = wales.record(r, n, &sys()).await.unwrap();
    assert_eq!(one.condition, "cancer");
    assert_eq!(one.condition_name.as_deref(), Some("Cancer"));
    assert_eq!(one.subtype.as_deref(), Some("04"));
    assert_eq!(
        one.subtype_scheme.as_deref(),
        Some("wales-primary-cancer-site")
    );
    assert_eq!(one.subtype_name.as_deref(), Some("Lung"));
    assert_eq!(one.profile, "wales");
    assert_eq!(one.days_waited, 3);
    assert_eq!(one.target_days, Some(62));
    assert!(one.hold_patient_cancel);
    assert_eq!(one.booking_priority, Some(Priority::Urgent));
    assert!(one.stop_reason.is_none());

    // The same condition twice on one referral would count one wait twice.
    let again = wales.record(r, new("cancer", Some("04"), 1), &sys()).await;
    assert!(
        again.is_err(),
        "a second open cancer suspicion on one referral is refused"
    );

    // Another condition on the same referral is fine; it has no subtypes and no target.
    let osa = wales
        .record(r, new("obstructive_sleep_apnoea", None, 10), &sys())
        .await
        .unwrap();
    assert_eq!(osa.subtype, None);
    assert_eq!(osa.target_days, None);
    assert!(!osa.hold_patient_cancel);

    // An optional subtype list: diabetes may name its type or not.
    let r2 = referral(&s, "wpas1-record").await;
    let t1 = wales
        .record(r2, new("diabetes", Some("type_1"), 1), &sys())
        .await
        .unwrap();
    assert_eq!(t1.subtype_name.as_deref(), Some("Type 1"));
    let r3 = referral(&s, "wpas1-record").await;
    assert!(
        wales
            .record(r3, new("diabetes", None, 1), &sys())
            .await
            .is_ok()
    );

    for (bad, why) in [
        (
            new("cancer", None, 1),
            "cancer without a site where the list is required",
        ),
        (new("cancer", Some("99"), 1), "a site not on the list"),
        (
            new("obstructive_sleep_apnoea", Some("08"), 1),
            "a subtype on a condition with none",
        ),
        (
            new("gout", None, 1),
            "a condition the profile does not define",
        ),
        (
            new("diabetes", None, -1),
            "a point of suspicion in the future",
        ),
        (
            NewSuspicion {
                suspected_by: "  ".into(),
                ..new("dementia", None, 1)
            },
            "no named clinician",
        ),
        (
            NewSuspicion {
                unique_pathway_identifier: Some("X".repeat(25)),
                ..new("parkinsons_disease", None, 1)
            },
            "a pathway identifier longer than 24",
        ),
    ] {
        let rx = referral(&s, "wpas1-record").await;
        assert!(wales.record(rx, bad, &sys()).await.is_err(), "{why}");
    }
    assert!(
        wales
            .record(Uuid::new_v4(), new("diabetes", None, 1), &sys())
            .await
            .is_err(),
        "an unknown referral"
    );

    let listed = wales.for_referral(r).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, osa.id, "oldest point of suspicion first");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_generic_profile_sets_no_target_and_names_no_national_list() {
    let s = state().await;
    let generic = service(&s, "generic");
    let r = referral(&s, "wpas8-generic").await;
    // Cancer with no site list: no site is asked for, and one given is refused.
    let c = generic
        .record(r, new("cancer", None, 80), &sys())
        .await
        .unwrap();
    assert_eq!(c.target_days, None);
    assert_eq!(
        c.target_status,
        TargetStatus::NoTarget,
        "80 days, and never past a target nobody set"
    );
    assert!(
        c.hold_patient_cancel,
        "the generic profile still holds a suspected cancer's cancel"
    );
    assert_eq!(c.booking_priority, None);
    let r2 = referral(&s, "wpas8-generic").await;
    assert!(
        generic
            .record(r2, new("cancer", Some("04"), 1), &sys())
            .await
            .is_err()
    );
    // Dementia, with its type.
    let d = generic
        .record(r2, new("dementia", Some("lewy_bodies"), 2), &sys())
        .await
        .unwrap();
    assert_eq!(d.subtype_name.as_deref(), Some("Dementia with Lewy bodies"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_rules_recorded_with_a_suspicion_outlive_a_change_of_profile() {
    let s = state().await;
    let r = referral(&s, "wpas8-snapshot").await;
    let x = service(&s, "wales")
        .record(r, new("cancer", Some("08"), 70), &sys())
        .await
        .unwrap();
    // Read back under another profile: the target, hold and priority it was recorded with stand, and the
    // codes are not relabelled with another list's words.
    let seen = service(&s, "generic").get(x.id).await.unwrap();
    assert_eq!(seen.target_days, Some(62));
    assert_eq!(seen.target_status, TargetStatus::Past);
    assert!(seen.hold_patient_cancel);
    assert_eq!(seen.profile, "wales");
    assert_eq!(seen.subtype.as_deref(), Some("08"));
    assert_eq!(
        seen.subtype_name, None,
        "the generic profile has no such list"
    );
    assert_eq!(
        seen.condition_name, None,
        "named only by the profile it was recorded under"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_wait_never_pauses_and_counts_real_days() {
    let s = state().await;
    let wales = service(&s, "wales");
    let r = referral(&s, "wpas1-wait").await;
    let x = wales
        .record(r, new("cancer", Some("08"), 40), &sys())
        .await
        .unwrap();
    assert_eq!(x.days_waited, 40);

    // Nothing that happens to the referral pauses or ends the wait: book it, close it.
    let sched = common::new_schedule_for(&s.db, "wpas1-wait").await;
    common::free_slot_in_hours(&s.db, sched.id, 48).await;
    s.referrals.book_first_available(r, &sys()).await.unwrap();
    s.referrals.close(r, Some("seen"), &sys()).await.unwrap();
    let after = wales.get(x.id).await.unwrap();
    assert_eq!(after.days_waited, 40, "the same real days");
    assert!(
        after.stop_reason.is_none(),
        "closing the referral does not stop the clock"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_clock_stops_once_and_never_before_it_started() {
    let s = state().await;
    let wales = service(&s, "wales");
    let r = referral(&s, "wpas1-stop").await;
    let x = wales
        .record(r, new("cancer", Some("03"), 30), &sys())
        .await
        .unwrap();
    let start = x.suspected_on;

    for (reason, on, by, why) in [
        (
            "treatment_started",
            start - Duration::days(1),
            "Dr A Bevan",
            "before the point of suspicion",
        ),
        (
            "treatment_started",
            today() + Duration::days(1),
            "Dr A Bevan",
            "in the future",
        ),
        ("cured", start, "Dr A Bevan", "a reason not on the list"),
        ("treatment_started", start, "  ", "no named person"),
    ] {
        assert!(
            wales
                .stop_if_unchanged(x.id, reason, on, by, None, &sys())
                .await
                .is_err(),
            "{why}"
        );
    }

    let stopped = wales
        .stop_if_unchanged(
            x.id,
            "treatment_started",
            start + Duration::days(20),
            "Dr A Bevan",
            None,
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(stopped.stop_reason, Some(StopReason::TreatmentStarted));
    assert_eq!(
        stopped.days_waited, 20,
        "judged on the stop date, not today"
    );
    assert_eq!(stopped.target_status, TargetStatus::Within);

    // Stopped is final.
    assert!(
        wales
            .stop_if_unchanged(
                x.id,
                "died",
                start + Duration::days(25),
                "Dr A Bevan",
                None,
                &sys()
            )
            .await
            .is_err()
    );
    // And the database holds it whole: no half-stopped rows.
    let half =
        s.db.run(Stmt::new(
            "UPDATE suspected_conditions SET stopped_on = NULL WHERE id = $1",
            [x.id.into()],
        ))
        .await;
    assert!(
        half.is_err(),
        "a stop reason without a date is refused by the database"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_open_list_orders_by_wait_and_marks_the_target() {
    let s = state().await;
    let wales = service(&s, "wales");
    let past = wales
        .record(
            referral(&s, "wpas1-list").await,
            new("cancer", Some("10"), 70),
            &sys(),
        )
        .await
        .unwrap();
    let near = wales
        .record(
            referral(&s, "wpas1-list").await,
            new("cancer", Some("10"), 58),
            &sys(),
        )
        .await
        .unwrap();
    let within = wales
        .record(
            referral(&s, "wpas1-list").await,
            new("cancer", Some("10"), 5),
            &sys(),
        )
        .await
        .unwrap();
    let no_target = wales
        .record(
            referral(&s, "wpas1-list").await,
            new("dementia", None, 400),
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(past.target_status, TargetStatus::Past);
    assert_eq!(near.target_status, TargetStatus::Near);
    assert_eq!(within.target_status, TargetStatus::Within);
    assert_eq!(
        no_target.target_status,
        TargetStatus::NoTarget,
        "no invented target"
    );
    assert_eq!(no_target.target_days, None);

    let all = common::every_page(|rows| wales.open(None, None, rows)).await;
    let pos = |id: Uuid| all.iter().position(|x| x.id == id).unwrap();
    assert!(
        pos(no_target.id) < pos(past.id),
        "longest wait first, whatever the condition"
    );
    assert!(pos(past.id) < pos(near.id) && pos(near.id) < pos(within.id));
    let mut last = i64::MAX;
    for x in &all {
        assert!(x.days_waited <= last, "ordered by days waited");
        last = x.days_waited;
        assert!(x.stop_reason.is_none(), "only open suspicions");
    }

    // Filtered to one subtype, and a stopped one leaves the list.
    let urology = common::every_page(|rows| wales.open(Some("cancer"), Some("10"), rows)).await;
    assert!(urology.iter().all(|x| x.subtype.as_deref() == Some("10")));
    assert!(urology.iter().any(|x| x.id == near.id));
    wales
        .stop_if_unchanged(near.id, "downgraded", today(), "Dr A Bevan", None, &sys())
        .await
        .unwrap();
    let after = common::every_page(|rows| wales.open(None, None, rows)).await;
    assert!(after.iter().all(|x| x.id != near.id));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_report_counts_each_stopped_suspicion_once() {
    let s = state().await;
    let wales = service(&s, "wales");
    // A day well in the past that no other test stops a clock on.
    let base = NaiveDate::from_ymd_opt(2001, 1, 1).unwrap()
        + Duration::days(i64::from(Uuid::new_v4().as_u128() as u16 % 3000));
    let stop = |days: i64| base + Duration::days(days);
    let cell = |r: &patient_administration_system::suspicions::SuspicionReport| {
        r.rows
            .iter()
            .find(|x| {
                x.condition == "cancer"
                    && x.subtype.as_deref() == Some("05")
                    && x.profile == "wales"
            })
            .cloned()
    };
    let n0 = cell(&wales.report(stop(60), stop(80)).await.unwrap()).map_or(0, |x| x.stopped);

    // Three sarcoma suspicions stopped in the window (two within 62 days, one past), one entered in error.
    for (waited, reason) in [
        (60, "treatment_started"),
        (62, "downgraded"),
        (63, "died"),
        (61, "entered_in_error"),
    ] {
        let r = referral(&s, "wpas1-report").await;
        let mut n = new("cancer", Some("05"), 0);
        n.suspected_on = base + Duration::days(80 - waited);
        let x = wales.record(r, n, &sys()).await.unwrap();
        s.db.run(Stmt::new(
            "UPDATE suspected_conditions SET stop_reason = $2, stopped_on = $3, stopped_by = 'Dr A Bevan', stopped_at = now() WHERE id = $1",
            [x.id.into(), reason.into(), stop(80).into()],
        ))
        .await
        .unwrap();
    }
    let c = cell(&wales.report(stop(60), stop(80)).await.unwrap()).unwrap();
    assert_eq!(
        c.stopped - n0,
        3,
        "each stopped once; entered in error left out"
    );
    assert_eq!(c.target_days, Some(62));
    assert_eq!(c.condition_name.as_deref(), Some("Cancer"));
    if n0 == 0 {
        assert_eq!(c.within_target, Some(2));
        assert_eq!(c.median_days, Some(62.0));
    }
    assert!(
        wales.report(stop(10), stop(1)).await.is_err(),
        "from after to"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patients_cancel_is_held_when_the_condition_says_so() {
    use patient_administration_system::models::appointment::AppointmentStatus;
    use patient_administration_system::writeback::{
        HELD_FOR_STAFF, ReplyAction, ReplyRequest, ReplyStatus,
    };
    let s = state().await;
    let wales = service(&s, "wales");
    let service_name = format!("wpas1-held-{}", Uuid::new_v4());
    let sched = common::new_schedule_for(&s.db, &service_name).await;
    common::free_slot_in_hours(&s.db, sched.id, 72).await;
    common::free_slot_in_hours(&s.db, sched.id, 96).await;

    let cancel =
        |a: &patient_administration_system::models::appointment::Appointment| ReplyRequest {
            message_id: format!("m-{}", Uuid::new_v4()),
            action: ReplyAction::Cancel,
            appointment_id: a.id,
            person_id: a.person_id,
            channel: Some("sms".into()),
            barrier_code: None,
            new_slot_id: None,
            modality: None,
            received_at: None,
        };

    // A condition that holds a cancel: held, and a person is asked to phone.
    let r = referral(&s, &service_name).await;
    wales
        .record(r, new("cancer", Some("01"), 2), &sys())
        .await
        .unwrap();
    let a = s
        .referrals
        .book_first_available(r, &sys())
        .await
        .unwrap()
        .appointment;
    let res = s.writeback.apply(cancel(&a), &sys()).await.unwrap();
    assert_eq!(res.status, ReplyStatus::Rejected);
    assert_eq!(res.reject_reason.as_deref(), Some(HELD_FOR_STAFF));
    let still = patient_administration_system::db::repositories::appointment::AppointmentRepository::find_by_id(&s.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, AppointmentStatus::Booked, "not cancelled");
    assert!(common::call_is_open(&s.db, a.id, "urgent_pathway").await);

    // A condition that does not: the cancel goes through.
    let r2 = referral(&s, &service_name).await;
    wales
        .record(r2, new("diabetes", None, 2), &sys())
        .await
        .unwrap();
    let b = s
        .referrals
        .book_first_available(r2, &sys())
        .await
        .unwrap()
        .appointment;
    let res2 = s.writeback.apply(cancel(&b), &sys()).await.unwrap();
    assert_ne!(res2.reject_reason.as_deref(), Some(HELD_FOR_STAFF));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_suspicion_raises_the_booking_priority_and_never_lowers_it() {
    let s = state().await;
    let wales = service(&s, "wales");

    // Waitlisted first as routine, then a suspicion of cancer: the entry is raised to urgent, with an audit row.
    let r = referral(&s, "wpas8-priority").await;
    s.referrals
        .waitlist(r, Priority::Routine, &sys())
        .await
        .unwrap();
    wales
        .record(r, new("cancer", Some("04"), 1), &sys())
        .await
        .unwrap();
    let (entry, p) = entry_of(&s, r).await;
    assert_eq!(p, "urgent");
    assert_eq!(
        common::audit_count(&s.db, entry, "priority_raised").await,
        1,
        "the raise is audited"
    );

    // The suspicion first, then waitlisted as routine: put on as urgent.
    let r2 = referral(&s, "wpas8-priority").await;
    wales
        .record(r2, new("cancer", Some("04"), 1), &sys())
        .await
        .unwrap();
    s.referrals
        .waitlist(r2, Priority::Routine, &sys())
        .await
        .unwrap();
    assert_eq!(entry_of(&s, r2).await.1, "urgent");

    // Never lowered: an emergency stays an emergency.
    let r3 = referral(&s, "wpas8-priority").await;
    s.referrals
        .waitlist(r3, Priority::Emergency, &sys())
        .await
        .unwrap();
    wales
        .record(r3, new("cancer", Some("04"), 1), &sys())
        .await
        .unwrap();
    let (e3, p3) = entry_of(&s, r3).await;
    assert_eq!(p3, "emergency");
    assert_eq!(common::audit_count(&s.db, e3, "priority_raised").await, 0);

    // A condition with no booking priority changes nothing.
    let r4 = referral(&s, "wpas8-priority").await;
    s.referrals
        .waitlist(r4, Priority::Routine, &sys())
        .await
        .unwrap();
    wales
        .record(r4, new("dementia", None, 1), &sys())
        .await
        .unwrap();
    assert_eq!(entry_of(&s, r4).await.1, "routine");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn recording_and_stopping_audit_and_publish() {
    let s = state().await;
    let wales = service(&s, "wales");
    let r = referral(&s, "wpas1-audit").await;
    let x = wales
        .record(r, new("cardiovascular_disease", None, 4), &sys())
        .await
        .unwrap();
    assert_eq!(common::audit_count(&s.db, x.id, "recorded").await, 1);
    wales
        .stop_if_unchanged(
            x.id,
            "treatment_started",
            today(),
            "Dr A Bevan",
            None,
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(common::audit_count(&s.db, x.id, "stopped").await, 1);
    let events: i64 = s
        .db
        .one(Stmt::new(
            "SELECT count(*) AS n FROM outbox_events WHERE event_type IN ('SuspicionRecorded','SuspicionStopped') \
             AND payload->>'suspicion_id' = $1",
            [x.id.to_string().into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .col("n")
        .unwrap();
    assert_eq!(events, 2, "one event for each");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn stopping_needs_the_version_read() {
    let mut s = state().await;
    s.suspicions = Arc::new(service(&s, "wales"));
    let app = patient_administration_system::api::rest::router(s.clone());
    let r = referral(&s, "wpas1-http").await;

    let (st, _, v) = call(
        &app,
        "GET",
        "/api/suspicion-profile",
        None,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"]["name"], "wales");
    assert!(
        v["data"]["conditions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["code"] == "dementia")
    );

    let (st, _, v) = call(
        &app,
        "POST",
        &format!("/api/referrals/{r}/suspicions"),
        None,
        serde_json::json!({
            "condition": "cancer", "subtype": "08",
            "suspected_on": (Utc::now() - Duration::days(9)).date_naive(),
            "suspected_by": "Dr Rhian Evans", "recorded_by": "Gareth Jones"
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["condition"], "cancer");
    assert_eq!(v["data"]["subtype"], "08");
    assert_eq!(v["data"]["subtype_name"], "Breast");
    let id = v["data"]["id"].as_str().unwrap().to_string();

    let (st, tag, _) = call(
        &app,
        "GET",
        &format!("/api/suspicions/{id}"),
        None,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let tag = tag.expect("an ETag");

    let body = serde_json::json!({"reason": "downgraded", "stopped_on": today(), "stopped_by": "Dr A Bevan"});
    let uri = format!("/api/suspicions/{id}/stop");
    let (st, _, _) = call(&app, "POST", &uri, None, body.clone()).await;
    assert_eq!(st, StatusCode::PRECONDITION_REQUIRED);
    let (st, _, _) = call(
        &app,
        "POST",
        &uri,
        Some("\"2000-01-01T00:00:00.000000Z\""),
        body.clone(),
    )
    .await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (st, _, v) = call(&app, "POST", &uri, Some(&tag), body.clone()).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["stop_reason"], "downgraded");
    let (st, _, _) = call(&app, "POST", &uri, Some("*"), body).await;
    assert_eq!(st, StatusCode::CONFLICT, "stopped once");

    // The list and report routes answer, and a condition the profile does not define is a 400.
    let (st, _, _) = call(
        &app,
        "GET",
        "/api/suspicions?condition=cancer",
        None,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = call(
        &app,
        "GET",
        "/api/suspicions?condition=gout",
        None,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _, v) = call(
        &app,
        "GET",
        &format!(
            "/api/suspicions/report?from={}&to={}",
            today() - Duration::days(30),
            today()
        ),
        None,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _, v) = call(
        &app,
        "GET",
        &format!("/api/referrals/{r}/suspicions"),
        None,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["data"].as_array().unwrap().len(), 1);
}
