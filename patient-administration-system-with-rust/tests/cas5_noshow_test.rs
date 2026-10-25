//! CAP-12 / CAS-5: the no-show probability. Support only, interpretable, and gated by people.
//! Needs PostgreSQL: `cargo test -- --ignored --test-threads=1` (tests set environment variables, and the
//! approvals they record are removed again at the end of each).

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::standby::{CandidateFilter, StandbyInput};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

const NOTES: &str = "Backtested on twelve months by quintile and language; the gap between groups is within what we accept.";

fn set(k: &str, v: &str) {
    // SAFETY: these tests run single-threaded (see the header).
    unsafe { std::env::set_var(k, v) };
}
fn unset(k: &str) {
    // SAFETY: as above.
    unsafe { std::env::remove_var(k) };
}

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

async fn clear_reviews(s: &AppState) {
    s.db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        "DELETE FROM noshow_reviews".to_string(),
    ))
    .await
    .unwrap();
}

async fn approve_both(s: &AppState) {
    for kind in ["fairness", "clinical_safety"] {
        s.noshow
            .record_review(kind, "Dr A. Reviewer", "approved", NOTES, &sys())
            .await
            .unwrap();
    }
}

/// Everything the tests set, put back.
async fn reset(s: &AppState) {
    for k in [
        "PAS_NOSHOW_SCORE",
        "PAS_NOSHOW_WEIGHTS",
        "PAS_NOSHOW_CALL_CAPACITY",
        "PAS_NOSHOW_CALL_HORIZON_DAYS",
        "PAS_NOSHOW_SHOW_PERCENT",
        "PAS_NOSHOW_MEDIUM",
    ] {
        unset(k);
    }
    clear_reviews(s).await;
    // Leave nothing behind for other suites: the staff call list shows the earliest 500, and these are tasks.
    for sql in [
        "DELETE FROM staff_call_tasks WHERE reason = 'high_risk'",
        "DELETE FROM noshow_assessments",
        "DELETE FROM noshow_rollout WHERE service_type = 'test-clinic'",
    ] {
        s.db.execute_raw(Statement::from_string(DbBackend::Postgres, sql.to_string()))
            .await
            .unwrap();
    }
}

/// The shared test clinic is moved straight to live (the shadow-period rule has its own test).
async fn go_live(s: &AppState) {
    s.db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        "INSERT INTO noshow_rollout (service_type, stage, changed_by, note, live_since) \
         VALUES ('test-clinic','live','test','test',now()) \
         ON CONFLICT (service_type) DO UPDATE SET stage='live', live_since=now()"
            .to_string(),
    ))
    .await
    .unwrap();
}

async fn booked(s: &AppState, person: Uuid, days: i64) -> Appointment {
    let schedule = common::new_schedule(&s.db).await.id;
    let slot = common::free_slot(&s.db, schedule, days).await;
    s.scheduling
        .book_slot(slot.id, person, &sys())
        .await
        .unwrap()
}

/// `n` past appointments for the patient with the given status.
async fn history(s: &AppState, person: Uuid, status: &str, n: i64) {
    for i in 0..n {
        let start = Utc::now() - Duration::days(10 + i);
        let mut a = Appointment::new(person, start, start + Duration::minutes(15));
        a.status = AppointmentStatus::Booked;
        let a = AppointmentRepository::create(&s.db, &a).await.unwrap();
        s.db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE appointments SET status = $2 WHERE id = $1",
            [a.id.into(), status.into()],
        ))
        .await
        .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn it_is_off_until_it_is_switched_on_and_a_named_person_has_approved_fairness_and_safety() {
    let s = state().await;
    reset(&s).await;
    let a = booked(&s, Uuid::new_v4(), 20).await;

    let st = s.noshow.status().await.unwrap();
    assert!(!st.active && st.reason.unwrap().contains("PAS_NOSHOW_SCORE is not true"));
    assert!(
        s.noshow
            .for_appointment(a.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("switched off")
    );
    assert!(s.noshow.recalculate_all().await.is_err());
    assert!(s.noshow.priority_list(10).await.is_err());

    set("PAS_NOSHOW_SCORE", "true");
    assert!(
        !s.noshow.status().await.unwrap().active,
        "the switch alone is not enough"
    );
    // A named reviewer and real notes are needed to approve.
    assert!(
        s.noshow
            .record_review("fairness", "", "approved", NOTES, &sys())
            .await
            .is_err()
    );
    assert!(
        s.noshow
            .record_review("fairness", "Dr A", "approved", "looks fine", &sys())
            .await
            .is_err(),
        "notes say what was looked at"
    );
    assert!(
        s.noshow
            .record_review("vibes", "Dr A", "approved", NOTES, &sys())
            .await
            .is_err()
    );
    s.noshow
        .record_review("fairness", "Dr A. Reviewer", "approved", NOTES, &sys())
        .await
        .unwrap();
    let st = s.noshow.status().await.unwrap();
    assert!(
        st.fairness_approved && !st.clinical_safety_approved && !st.active,
        "fairness alone is not enough"
    );
    s.noshow
        .record_review("clinical_safety", "Dr B. Safety", "approved", NOTES, &sys())
        .await
        .unwrap();
    assert!(s.noshow.status().await.unwrap().active);
    s.noshow.for_appointment(a.id).await.unwrap();

    // A later rejection switches it off at once.
    s.noshow
        .record_review(
            "fairness",
            "Dr C",
            "rejected",
            "Quintile 5 is flagged high at twice the rate it misses.",
            &sys(),
        )
        .await
        .unwrap();
    let st = s.noshow.status().await.unwrap();
    assert!(!st.active && !st.fairness_approved);
    assert!(s.noshow.for_appointment(a.id).await.is_err());

    // An approval that has gone stale (older than 180 days) no longer counts.
    reset(&s).await;
    set("PAS_NOSHOW_SCORE", "true");
    approve_both(&s).await;
    assert!(s.noshow.status().await.unwrap().active);
    s.db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        "UPDATE noshow_reviews SET reviewed_at = now() - interval '181 days' WHERE kind = 'fairness'".to_string(),
    ))
    .await
    .unwrap();
    assert!(
        !s.noshow.status().await.unwrap().active,
        "a fairness review lapses"
    );
    assert!(
        common::audit_count(
            &s.db,
            Uuid::new_v5(&Uuid::NAMESPACE_OID, b"weighted-sum-1"),
            "review_approved"
        )
        .await
            >= 2,
        "approvals are audited"
    );
    reset(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_new_booking_gets_a_probability_with_its_factors_and_a_rerun_the_same_day_changes_nothing()
 {
    let s = state().await;
    reset(&s).await;
    set("PAS_NOSHOW_SCORE", "true");
    approve_both(&s).await;
    let person = Uuid::new_v4();
    history(&s, person, "no_show", 3).await;
    history(&s, person, "fulfilled", 1).await;
    let a = booked(&s, person, 40).await; // the hook scores it on creation
    let shown = s.noshow.for_appointment(a.id).await.unwrap();
    assert_eq!(shown.sample_size, 4, "completed past visits only");
    let past = shown
        .factors
        .iter()
        .find(|f| f.name == "past_visits")
        .unwrap();
    assert!(past.detail.contains("missed 3 of 4"), "{}", past.detail);
    assert!(
        shown.probability > 0.30,
        "three misses in four is not low: {}",
        shown.probability
    );

    // A rerun the same day returns the same assessment, not a new one.
    let again = s.noshow.for_appointment(a.id).await.unwrap();
    assert_eq!(again.computed_at, shown.computed_at);
    let n1 = s.noshow.recalculate_all().await.unwrap();
    let n2 = s.noshow.recalculate_all().await.unwrap();
    assert!(n1 >= 1 && n1 == n2);
    assert_eq!(
        s.noshow.for_appointment(a.id).await.unwrap().computed_at,
        shown.computed_at,
        "idempotent within a day"
    );

    // Yesterday's assessment is recomputed today.
    s.db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE noshow_assessments SET computed_on = computed_on - 1 WHERE appointment_id = $1",
        [a.id.into()],
    ))
    .await
    .unwrap();
    assert!(s.noshow.for_appointment(a.id).await.unwrap().computed_at > shown.computed_at);
    reset(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn changing_a_threshold_changes_the_band_without_recomputing_and_the_percentage_is_hidden_until_calibrated()
 {
    let s = state().await;
    reset(&s).await;
    set("PAS_NOSHOW_SCORE", "true");
    set(
        "PAS_NOSHOW_WEIGHTS",
        r#"{"base":0.50,"history":0,"lead":0,"age":{}}"#,
    );
    approve_both(&s).await;
    let a = booked(&s, Uuid::new_v4(), 30).await;
    let first = s.noshow.for_appointment(a.id).await.unwrap();
    assert_eq!(first.probability, 0.50);
    assert_eq!(
        serde_json::to_value(first.band).unwrap(),
        "medium",
        "above 40"
    );
    // Staff are shown the band and the reasons, not a number the weights cannot yet justify.
    assert_eq!(first.probability_percent, None);
    assert!(
        !serde_json::to_string(&first).unwrap().contains("probab"),
        "the number is not in the response at all"
    );
    set("PAS_NOSHOW_MEDIUM", "0.60");
    let after = s.noshow.for_appointment(a.id).await.unwrap();
    assert_eq!(serde_json::to_value(after.band).unwrap(), "low");
    assert_eq!(after.computed_at, first.computed_at, "not recomputed");
    // Only after calibration is someone to set the switch.
    set("PAS_NOSHOW_SHOW_PERCENT", "true");
    assert_eq!(
        s.noshow
            .for_appointment(a.id)
            .await
            .unwrap()
            .probability_percent,
        Some(50.0)
    );
    reset(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_call_list_is_the_top_few_matched_to_capacity_not_a_probability_cut_off() {
    let s = state().await;
    reset(&s).await;
    set("PAS_NOSHOW_SCORE", "true");
    approve_both(&s).await;
    set("PAS_NOSHOW_CALL_CAPACITY", "2");
    go_live(&s).await;

    // Three in the coming week with different probabilities, and one far beyond the horizon.
    let mut ids = Vec::new();
    for (base, days) in [(0.30, 3), (0.50, 4), (0.80, 5), (0.95, 30)] {
        set(
            "PAS_NOSHOW_WEIGHTS",
            &format!(r#"{{"base":{base},"history":0,"lead":0,"age":{{}}}}"#),
        );
        let a = booked(&s, Uuid::new_v4(), days).await;
        s.noshow.for_appointment(a.id).await.unwrap();
        ids.push(a.id.to_string());
    }
    assert_eq!(s.noshow.refresh_call_list().await.unwrap(), 2);
    let list = s.noshow.priority_list(500).await.unwrap();
    let listed: Vec<String> = list.iter().map(|x| x.appointment_id.to_string()).collect();
    assert_eq!(
        listed,
        vec![ids[2].clone(), ids[1].clone()],
        "the top two, highest first"
    );
    assert_eq!(
        s.noshow.refresh_call_list().await.unwrap(),
        0,
        "idempotent, and at capacity"
    );
    let item = &list[0];
    assert!(item.factors.len() >= 4, "the list says why");
    assert!(
        item.probability_percent.is_none(),
        "no number while uncalibrated"
    );
    // Listed means high; the third is low (0.30 is under the medium line), the far one is not on the list.
    let band_of = |id: &str| {
        let s = s.clone();
        let id: Uuid = id.parse().unwrap();
        async move { serde_json::to_value(s.noshow.for_appointment(id).await.unwrap().band).unwrap() }
    };
    assert_eq!(band_of(&ids[2]).await, "high");
    assert_eq!(band_of(&ids[0]).await, "low");

    // A person rings and records an outcome: capacity frees and the next one is listed.
    let task: Uuid = item.task_id;
    assert!(
        s.ladder
            .complete_call(task, "reached", Some("Nurse B"))
            .await
            .unwrap()
    );
    assert_eq!(s.noshow.refresh_call_list().await.unwrap(), 1);
    let list = s.noshow.priority_list(500).await.unwrap();
    assert!(
        list.iter().any(|x| x.appointment_id.to_string() == ids[0]),
        "{list:?}"
    );
    assert!(
        list.iter().all(|x| x.appointment_id.to_string() != ids[3]),
        "beyond the horizon"
    );

    // Zero capacity lists nobody.
    set("PAS_NOSHOW_CALL_CAPACITY", "0");
    s.db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        "DELETE FROM staff_call_tasks WHERE reason = 'high_risk'".to_string(),
    ))
    .await
    .unwrap();
    assert_eq!(s.noshow.refresh_call_list().await.unwrap(), 0);
    reset(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn age_is_not_an_input_by_default_and_a_fairness_approval_must_justify_it_if_it_is() {
    let s = state().await;
    reset(&s).await;
    let w = patient_administration_system::noshow::Weights::default();
    assert!(w.age.is_empty(), "no age weights by default");
    assert!(!s.noshow.status().await.unwrap().age_in_use);

    set("PAS_NOSHOW_WEIGHTS", r#"{"age":{"16-24":0.03}}"#);
    assert!(s.noshow.status().await.unwrap().age_in_use);
    let e = s
        .noshow
        .record_review("fairness", "Dr A. Reviewer", "approved", NOTES, &sys())
        .await
        .unwrap_err();
    assert!(e.to_string().contains("age"), "{e}");
    s.noshow
        .record_review(
            "fairness",
            "Dr A. Reviewer",
            "approved",
            "Reviewed the audit. Age bands add 3 points for 16-24 because the audit shows their miss rate is higher in this clinic; accepted.",
            &sys(),
        )
        .await
        .unwrap();
    reset(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn with_scores_present_rebooking_standby_and_offers_behave_exactly_as_without() {
    // NFR-11: the score only adds a task for a person; it takes no part in what anyone is offered.
    let s = state().await;
    reset(&s).await;
    let service = format!("svc-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    let person = Uuid::new_v4();
    let own = common::free_slot(&s.db, schedule, 5).await;
    let appt = s
        .scheduling
        .book_slot(own.id, person, &sys())
        .await
        .unwrap();
    for d in [6, 7, 8, 9] {
        common::free_slot(&s.db, schedule, d).await;
    }
    let offered = |o: patient_administration_system::rebooking::Offer| {
        o.options.into_iter().map(|x| x.slot_id).collect::<Vec<_>>()
    };
    let before = offered(s.rebooking.offer(appt.id).await.unwrap());
    assert!(!before.is_empty());

    // Standby: three people, in the order they joined.
    let canceller = Uuid::new_v4();
    let slot = common::free_slot(&s.db, schedule, 12).await;
    let b = s
        .scheduling
        .book_slot(slot.id, canceller, &sys())
        .await
        .unwrap();
    let people: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
    for p in &people {
        s.standby
            .add(
                StandbyInput {
                    person_id: *p,
                    service_type: service.clone(),
                    short_notice_ok: true,
                    language: None,
                    needs_transport: false,
                    phone_only: false,
                },
                &sys(),
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    s.scheduling
        .cancel(
            b.id,
            patient_administration_system::models::appointment::CancellationReason::PatientRequest,
            &sys(),
        )
        .await
        .unwrap();
    let rid: Uuid =
        s.db.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT id FROM slot_releases WHERE slot_id = $1 ORDER BY released_at DESC LIMIT 1",
            [slot.id.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "id")
        .unwrap();
    let order = |v: Vec<patient_administration_system::standby::StandbyEntry>| {
        v.into_iter().map(|e| e.person_id).collect::<Vec<_>>()
    };
    let standby_before = order(
        s.standby
            .candidates(rid, &CandidateFilter::default())
            .await
            .unwrap(),
    );
    assert_eq!(standby_before, people);

    // Now switch the score on and give everyone concerned a high probability and a task.
    set("PAS_NOSHOW_SCORE", "true");
    set(
        "PAS_NOSHOW_WEIGHTS",
        r#"{"base":0.90,"history":0,"lead":0,"age":{}}"#,
    );
    approve_both(&s).await;
    s.noshow.for_appointment(appt.id).await.unwrap();
    for p in &people {
        let a = booked(&s, *p, 60).await; // each standby patient has another booked appointment, scored high
        assert!(s.noshow.for_appointment(a.id).await.unwrap().probability > 0.70);
    }
    s.db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "DELETE FROM rebook_offers WHERE appointment_id = $1",
        [appt.id.into()],
    ))
    .await
    .unwrap();
    let after = offered(s.rebooking.offer(appt.id).await.unwrap());
    assert_eq!(before, after, "rebooking options are the same");
    let standby_after = order(
        s.standby
            .candidates(rid, &CandidateFilter::default())
            .await
            .unwrap(),
    );
    assert_eq!(
        standby_before, standby_after,
        "standby candidates are the same, in the same order"
    );
    // And the slot is still free to whoever is next: nothing was held back.
    let own_after =
        patient_administration_system::db::repositories::slot::SlotRepository::find_by_id(
            &s.db, slot.id,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        own_after.status,
        patient_administration_system::models::schedule::SlotStatus::Free
    );
    reset(&s).await;
}

#[test]
fn test_the_scoring_module_never_writes_to_slots_offers_standby_or_appointments() {
    // A crude, blunt guard: the module's source never names the tables that decide what a patient is offered,
    // and never changes an appointment, so it cannot hold back, reorder or remove a slot.
    // Every file of the module (it is split into several): a guard that read only `mod.rs` would check a fraction.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/noshow");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 7,
        "expected the whole module, found {files:?}"
    );
    let src: String = files
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "UPDATE slots",
        "INSERT INTO slots",
        "DELETE FROM slots",
        "slot_offers",
        "slot_releases",
        "standby_entries",
        "rebook_offers",
        "UPDATE appointments",
        "INSERT INTO appointments",
        "DELETE FROM appointments",
        "StandbyService",
        "RebookService",
        "SchedulingService",
    ] {
        assert!(
            !src.contains(forbidden),
            "the score must not touch {forbidden}"
        );
    }
    // What it does write: its own tables and the staff task list.
    assert!(
        src.contains("INSERT INTO noshow_assessments")
            && src.contains("INSERT INTO staff_call_tasks")
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_fairness_audit_backtests_by_quintile_and_language_withholds_small_cells_and_works_while_off()
 {
    let s = state().await;
    reset(&s).await;
    assert!(
        !s.noshow.status().await.unwrap().active,
        "the evidence is available before approval"
    );
    let lang = format!("x{}", &Uuid::new_v4().simple().to_string()[..6]);
    // 14 patients in quintile 1 who all attend; 12 in quintile 5 who all miss, each with the same history.
    for (q, missed, n) in [
        (1i16, false, 14),
        (5, true, 12),
        (3, true, 3),
        (2, false, 2),
    ] {
        for _ in 0..n {
            let person = Uuid::new_v4();
            s.analytics
                .set_demographics(person, Some("45-54"), Some(q), "ETL", &sys())
                .await
                .unwrap();
            s.db.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO patient_communication_profiles (person_id, preferred_language) VALUES ($1,$2)",
                [person.into(), lang.clone().into()],
            ))
            .await
            .unwrap();
            history(&s, person, if missed { "no_show" } else { "fulfilled" }, 1).await;
        }
    }
    let audit = s.noshow.fairness_audit(365, Some(1)).await.unwrap();
    assert_eq!(audit["k"], 10, "k cannot be lowered");
    let cells = audit["cells"].as_array().unwrap();
    let cell = |q: &str| {
        cells
            .iter()
            .find(|c| c["deprivation_quintile"] == q && c["language"] == lang.as_str())
            .unwrap()
    };
    let q1 = cell("1");
    assert_eq!(q1["suppressed"], false);
    assert_eq!(q1["observed_no_show_rate"], 0.0);
    let q5 = cell("5");
    assert_eq!(q5["observed_no_show_rate"], 1.0);
    assert!(
        q5["mean_predicted"].as_f64().unwrap() < 1.0
            && q5["calibration_gap"].as_f64().unwrap() < 0.0,
        "a plain model under-predicts a group that always misses"
    );
    assert_eq!(cell("3")["suppressed"], true, "three is too few to show");
    assert_eq!(
        cell("2")["suppressed"],
        true,
        "so is two (and two hidden cells means no secondary suppression)"
    );
    assert!(
        cell("3").get("mean_predicted").is_none(),
        "a withheld cell shows nothing"
    );
    assert!(
        audit["how_to_read"]
            .as_str()
            .unwrap()
            .contains("Support only")
    );
    let json = audit.to_string();
    assert!(!json.contains("person_id"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_http_endpoints_report_status_the_card_and_refuse_while_off() {
    let s = state().await;
    reset(&s).await;
    let a = booked(&s, Uuid::new_v4(), 20).await;
    let app = patient_administration_system::api::rest::router(s.clone());
    let call = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let mut r = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    r = r.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(r.body(body).unwrap()).await.unwrap();
            let st = r.status();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(
                    &to_bytes(r.into_body(), 4 << 20).await.unwrap(),
                )
                .unwrap_or_default(),
            )
        }
    };
    let (st, v) = call("GET", "/api/noshow/status".into(), None).await;
    assert_eq!(
        (st, v["data"]["active"].as_bool()),
        (StatusCode::OK, Some(false))
    );
    let (st, v) = call("GET", "/api/noshow/model-card".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        v["data"]["never"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str().unwrap().contains("overbook"))
    );
    let (st, v) = call("GET", format!("/api/appointments/{}/noshow", a.id), None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    let (st, _) = call("POST", "/api/noshow/reviews".into(), Some(serde_json::json!({ "kind": "fairness", "reviewer": "", "outcome": "approved", "notes": "x" }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call("GET", "/api/noshow/fairness-audit?days=30".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call("GET", "/api/noshow/priority-list".into(), None).await;
    assert_eq!(st, StatusCode::CONFLICT);
    reset(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_schedule_view_can_ask_for_many_stored_assessments_at_once_and_gets_nothing_while_off() {
    let s = state().await;
    reset(&s).await;
    let (a, b) = (
        booked(&s, Uuid::new_v4(), 21).await,
        booked(&s, Uuid::new_v4(), 22).await,
    );
    let (active, items) = s.noshow.for_appointments(&[a.id, b.id]).await.unwrap();
    assert!(
        !active && items.is_empty(),
        "off: nothing, and not an error"
    );

    set("PAS_NOSHOW_SCORE", "true");
    approve_both(&s).await;
    // A clinic in shadow mode shows staff nothing, though it records.
    s.noshow.for_appointment(a.id).await.unwrap();
    let (active, items) = s.noshow.for_appointments(&[a.id, b.id]).await.unwrap();
    assert!(active && items.is_empty(), "shadow: recorded, not shown");
    assert!(s.noshow.for_staff(a.id).await.is_err());
    go_live(&s).await;
    assert!(s.noshow.for_staff(a.id).await.is_ok());
    // The second has not been reached by the daily pass, and asking does not score it.
    let (active, items) = s.noshow.for_appointments(&[a.id, b.id]).await.unwrap();
    assert!(active);
    assert_eq!(items.len(), 2.min(items.len()).max(1));
    assert!(items.iter().any(|x| x.appointment_id == a.id));
    reset(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_clinic_goes_live_only_after_enough_shadow_data_by_a_named_person_and_can_go_back() {
    let s = state().await;
    reset(&s).await;
    set("PAS_NOSHOW_SCORE", "true");
    approve_both(&s).await;
    set("PAS_NOSHOW_CALL_CAPACITY", "5");
    let a = booked(&s, Uuid::new_v4(), 3).await;
    s.noshow.for_appointment(a.id).await.unwrap();

    // Everything starts in shadow: no call, nothing shown, and the clinic says so.
    assert_eq!(
        s.noshow.refresh_call_list().await.unwrap(),
        0,
        "shadow creates no calls"
    );
    let r = s.noshow.rollout().await.unwrap();
    let mine = r.iter().find(|x| x.service_type == "test-clinic").unwrap();
    assert_eq!((mine.stage.as_str(), mine.may_go_live), ("shadow", false));

    // Too early: refused with the reason. A note and a name are needed too.
    let why = "The first clinic, after the audit was read.";
    let e = s
        .noshow
        .set_stage("test-clinic", "live", "Dr A", why, &sys())
        .await
        .unwrap_err();
    assert!(e.to_string().contains("days of shadow data"), "{e}");
    assert!(
        s.noshow
            .set_stage("test-clinic", "live", " ", why, &sys())
            .await
            .is_err()
    );
    assert!(
        s.noshow
            .set_stage("test-clinic", "live", "Dr A", "now", &sys())
            .await
            .is_err()
    );
    assert!(
        s.noshow
            .set_stage("test-clinic", "maybe", "Dr A", why, &sys())
            .await
            .is_err()
    );

    // After the shadow period it may go live, and then calls appear for that clinic only.
    s.db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        "UPDATE noshow_assessments SET computed_at = now() - interval '30 days'".to_string(),
    ))
    .await
    .unwrap();
    s.noshow
        .set_stage("test-clinic", "live", "Dr A", why, &sys())
        .await
        .unwrap();
    assert_eq!(s.noshow.refresh_call_list().await.unwrap(), 1);
    assert!(s.noshow.for_staff(a.id).await.is_ok());

    // It also needs the score to be active: a rejection stops going live as well as everything else.
    s.noshow
        .set_stage(
            "test-clinic",
            "shadow",
            "Dr A",
            "Back while we look at the audit.",
            &sys(),
        )
        .await
        .unwrap();
    assert!(s.noshow.for_staff(a.id).await.is_err(), "back in shadow");
    s.noshow
        .record_review(
            "fairness",
            "Dr C",
            "rejected",
            "Not acceptable for quintile 5.",
            &sys(),
        )
        .await
        .unwrap();
    assert!(
        s.noshow
            .set_stage("test-clinic", "live", "Dr A", why, &sys())
            .await
            .is_err()
    );
    reset(&s).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn the_evaluation_reports_calibration_and_before_after_against_clinics_still_in_shadow() {
    use chrono::Duration;
    let s = state().await;
    reset(&s).await;
    set("PAS_NOSHOW_SCORE", "true");
    set(
        "PAS_NOSHOW_WEIGHTS",
        r#"{"base":0.30,"history":0,"lead":0,"age":{}}"#,
    );
    approve_both(&s).await;
    go_live(&s).await;
    s.db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        "UPDATE noshow_rollout SET live_since = now() - interval '20 days' WHERE service_type = 'test-clinic'".to_string(),
    ))
    .await
    .unwrap();
    // 30 past appointments at the live clinic after it went live, 10 missed; scored at 0.30 beforehand.
    let schedule = common::new_schedule(&s.db).await.id;
    for i in 0..30 {
        let slot = common::free_slot(&s.db, schedule, 40 + i).await;
        let a = s
            .scheduling
            .book_slot(slot.id, Uuid::new_v4(), &sys())
            .await
            .unwrap();
        s.noshow.for_appointment(a.id).await.unwrap();
        let status = if i < 10 { "no_show" } else { "fulfilled" };
        s.db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE appointments SET status = $2, start_datetime = now() - make_interval(days => $3::int) WHERE id = $1",
            [a.id.into(), status.into(), (1 + (i % 15) as i32).into()],
        ))
        .await
        .unwrap();
    }
    let _ = Duration::zero();
    let v = s.noshow.evaluation(180, None).await.unwrap();
    let cal: Vec<&serde_json::Value> = v["calibration"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["predicted_tenth"] == "3")
        .collect();
    assert_eq!(cal.len(), 1);
    let c = cal[0];
    assert!(
        c["suppressed"] == false && c["appointments"].as_i64().unwrap() >= 30,
        "{c}"
    );
    assert!((c["mean_predicted"].as_f64().unwrap() - 0.30).abs() < 0.01);
    let clinic = v["clinics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["service_type"] == "test-clinic")
        .unwrap();
    assert!(
        clinic["after"]["no_show_rate"].as_f64().is_some(),
        "{clinic}"
    );
    assert!(clinic["live_since"].is_string());
    // No identifier anywhere in the evaluation.
    let text = v.to_string();
    assert!(!text.contains("person_id") && !text.contains("appointment_id"));
    reset(&s).await;
}
