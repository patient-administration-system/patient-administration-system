//! CAS-7: wording experiments. Needs PostgreSQL: `cargo test -- --ignored --test-threads=1`.
//! (One test runs an experiment on the real reminder, so run these single-threaded.)

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::ladder::LadderService;
use patient_administration_system::contact::ProfileInput;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::letter::LetterRepository;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::{DeliveryChannel, LetterTemplate};
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

type Log = Arc<Mutex<Vec<(String, String)>>>; // (to, body)

struct Capture(Log);

#[async_trait]
impl MessageSender for Capture {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _s: &str,
        body: &str,
    ) -> patient_administration_system::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((to.to_string(), body.to_string()));
        Ok(())
    }
}

/// A complete template group (English and Welsh) whose bodies carry `marker`.
async fn group(s: &AppState, name: &str, marker: &str) {
    for (locale, word) in [("en-001", "EN"), ("cy-001", "CY")] {
        let mut t = LetterTemplate::new(
            name.into(),
            "Reminder".into(),
            format!("{marker}-{word} {{{{ appointment.when }}}}"),
        );
        t.locale = locale.into();
        t.channels = vec![DeliveryChannel::Sms];
        LetterRepository::create_template(&s.db, &t).await.unwrap();
    }
}

fn uniq(p: &str) -> String {
    format!("{p}-{}", &Uuid::new_v4().simple().to_string()[..10])
}

async fn state() -> AppState {
    common::build_state(&common::database_url().unwrap()).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_experiment_needs_two_to_eight_complete_named_variants_and_one_per_message() {
    let s = state().await;
    let (a, b) = (uniq("a"), uniq("b"));
    group(&s, &a, "A").await;
    group(&s, &b, "B").await;
    let name = uniq("msg");
    let two = || vec![("A".to_string(), a.clone()), ("B".to_string(), b.clone())];
    assert!(
        s.experiments
            .start(&name, vec![("A".into(), a.clone())], "Dr A", &sys())
            .await
            .is_err(),
        "one variant is not an experiment"
    );
    assert!(
        s.experiments
            .start(
                &name,
                vec![("A".into(), a.clone()), ("A".into(), b.clone())],
                "Dr A",
                &sys()
            )
            .await
            .is_err(),
        "names once each"
    );
    assert!(
        s.experiments
            .start(
                &name,
                vec![("a".into(), a.clone()), ("b".into(), b.clone())],
                "Dr A",
                &sys()
            )
            .await
            .is_err(),
        "capital letters"
    );
    assert!(
        s.experiments
            .start(
                &name,
                vec![("A".into(), a.clone()), ("B".into(), uniq("missing"))],
                "Dr A",
                &sys()
            )
            .await
            .is_err(),
        "no such group"
    );
    assert!(
        s.experiments
            .start(&name, two(), " ", &sys())
            .await
            .is_err(),
        "a named person starts it"
    );

    let id = s
        .experiments
        .start(&name, two(), "Dr A", &sys())
        .await
        .unwrap();
    assert!(
        s.experiments
            .start(&name, two(), "Dr A", &sys())
            .await
            .is_err(),
        "one running experiment per message"
    );
    assert_eq!(common::audit_count(&s.db, id, "started").await, 1);
    s.experiments.stop(id, "Dr A", &sys()).await.unwrap();
    assert!(
        s.experiments.stop(id, "Dr A", &sys()).await.is_err(),
        "stopped once"
    );
    // After stopping, the message can be tested again.
    s.experiments
        .start(&name, two(), "Dr A", &sys())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn each_appointment_gets_one_recorded_variant_in_its_own_language_and_keeps_it() {
    let log: Log = Arc::default();
    let stub = Arc::new(StubPersonClient::new());
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture(log.clone())));
    // Free the real reminder for this test, whatever an earlier run left.
    s.db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        "UPDATE message_experiments SET stopped_at = now(), stopped_by = 'test' WHERE template_name = 'appointment_reminder' AND stopped_at IS NULL".to_string(),
    ))
    .await
    .unwrap();
    let (b_name,) = (uniq("reminder-b"),);
    group(&s, &b_name, "VARIANT-B").await;

    // 24 patients (some Welsh), each with an appointment a year ahead within a few minutes of each other.
    let base: DateTime<Utc> = Utc::now()
        + Duration::days(900)
        + Duration::minutes((Uuid::new_v4().as_u128() % 1_000_000) as i64 * 40);
    let mut who: Vec<(Uuid, String, Uuid)> = Vec::new(); // (person, phone, appointment)
    let mut welsh = Vec::new();
    for i in 0..24 {
        let person = Uuid::new_v4();
        let phone =
            format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000).replace(' ', "");
        let mut data = serde_json::Map::new();
        data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
        data.insert(
            "telecom".into(),
            serde_json::json!([{ "system": "phone", "value": phone }]),
        );
        stub.insert(Person { id: person, data });
        let start = base + Duration::minutes(i);
        let mut a = Appointment::new(person, start, start + Duration::minutes(15));
        a.status = AppointmentStatus::Booked;
        let a = AppointmentRepository::create(&s.db, &a).await.unwrap();
        who.push((person, phone, a.id));
        if i % 4 == 0 {
            welsh.push(person);
        }
    }
    let s = s.with_person_client(stub);
    for p in &welsh {
        s.contact
            .put(
                *p,
                ProfileInput {
                    preferred_language: Some("cy-001".into()),
                    ..Default::default()
                },
                &sys(),
            )
            .await
            .unwrap();
    }
    let exp = s
        .experiments
        .start(
            "appointment_reminder",
            vec![
                ("A".into(), "appointment_reminder".into()),
                ("B".into(), b_name.clone()),
            ],
            "Dr A",
            &sys(),
        )
        .await
        .unwrap();
    let ladder = LadderService::new(s.db.clone(), s.communication.clone())
        .with_link_base(Some("https://pas.test".into()));
    let now = base + Duration::minutes(23) - Duration::days(7) + Duration::minutes(1);
    ladder.run_once(now).await.unwrap();

    let sent = log.lock().unwrap().clone();
    let mut a_count = 0;
    let mut b_count = 0;
    for (person, phone, appt) in &who {
        let body = &sent
            .iter()
            .find(|(to, _)| to == phone)
            .unwrap_or_else(|| panic!("nothing sent to {phone}"))
            .1;
        let is_b = body.contains("VARIANT-B");
        // Recorded, and equal to what was sent.
        let rec = s
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT variant FROM message_experiment_assignments WHERE experiment_id = $1 AND appointment_id = $2",
                [exp.into(), (*appt).into()],
            ))
            .await
            .unwrap()
            .expect("the assignment is recorded");
        assert_eq!(
            rec.try_get::<String>("", "variant").unwrap() == "B",
            is_b,
            "recorded variant matches the wording sent"
        );
        if is_b {
            b_count += 1;
            // In the patient's own language.
            let want = if welsh.contains(person) {
                "VARIANT-B-CY"
            } else {
                "VARIANT-B-EN"
            };
            assert!(body.contains(want), "{body}");
        } else {
            a_count += 1;
        }
    }
    assert!(
        a_count > 0 && b_count > 0,
        "both wordings were used: {a_count} A, {b_count} B"
    );
    assert_eq!(a_count + b_count, 24);

    // The same appointment resolves to the same wording however often it is asked.
    for (_, _, appt) in who.iter().take(6) {
        let first = patient_administration_system::experiments::resolve_name(
            &s.db,
            *appt,
            "appointment_reminder",
        )
        .await
        .unwrap();
        let again = patient_administration_system::experiments::resolve_name(
            &s.db,
            *appt,
            "appointment_reminder",
        )
        .await
        .unwrap();
        assert_eq!(first, again);
    }
    // After stopping, a new appointment gets the ordinary wording and the old ones keep their record.
    s.experiments.stop(exp, "Dr A", &sys()).await.unwrap();
    let fresh = patient_administration_system::experiments::resolve_name(
        &s.db,
        Uuid::new_v4(),
        "appointment_reminder",
    )
    .await
    .unwrap();
    assert_eq!(fresh, "appointment_reminder");
    let n =
        s.db.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT count(*) AS n FROM message_experiment_assignments WHERE experiment_id = $1",
            [exp.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "n")
        .unwrap();
    assert_eq!(n, 24);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_readout_splits_by_quintile_and_language_and_withholds_small_cells_without_identifiers()
{
    let s = state().await;
    let (a, b) = (uniq("a"), uniq("b"));
    group(&s, &a, "A").await;
    group(&s, &b, "B").await;
    let exp = s
        .experiments
        .start(
            &uniq("msg"),
            vec![("A".into(), a), ("B".into(), b)],
            "Dr A",
            &sys(),
        )
        .await
        .unwrap();
    let start = Utc::now() + Duration::days(30);
    let mut ids = Vec::new();
    // Variant A, quintile 1, English: 12 patients, 10 attended and 2 missed. Variant B: 12, 6 attended, 6 missed.
    // Plus 3 and 2 in quintile 5 (too few to show; two hidden cells, so no secondary suppression).
    for (variant, quintile, n, attended) in [
        ("A", 1i16, 12, 10),
        ("B", 1, 12, 6),
        ("A", 5, 3, 3),
        ("B", 5, 2, 1),
    ] {
        for i in 0..n {
            let person = Uuid::new_v4();
            let mut ap = Appointment::new(person, start, start + Duration::minutes(15));
            ap.status = AppointmentStatus::Booked;
            let ap = AppointmentRepository::create(&s.db, &ap).await.unwrap();
            ids.push((person, ap.id));
            s.analytics
                .set_demographics(person, Some("45-54"), Some(quintile), "ETL", &sys())
                .await
                .unwrap();
            s.db.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO patient_communication_profiles (person_id, preferred_language) VALUES ($1,'en-001')",
                [person.into()],
            ))
            .await
            .unwrap();
            s.db.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO message_experiment_assignments (experiment_id, appointment_id, variant) VALUES ($1,$2,$3)",
                [exp.into(), ap.id.into(), variant.into()],
            ))
            .await
            .unwrap();
            let status = if i < attended { "fulfilled" } else { "no_show" };
            s.db.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE appointments SET status = $2 WHERE id = $1",
                [ap.id.into(), status.into()],
            ))
            .await
            .unwrap();
        }
    }
    let r = s.experiments.readout(exp, Some(1)).await.unwrap();
    assert_eq!(r.k, 10, "k cannot be lowered");
    assert_eq!(r.total_assigned, 29);
    let cell = |v: &str, q: &str| {
        r.cells
            .iter()
            .find(|c| c.variant == v && c.deprivation_quintile == q && c.language == "en-001")
            .unwrap()
    };
    let a1 = cell("A", "1");
    assert_eq!(
        (
            a1.assigned,
            a1.attended,
            a1.did_not_attend,
            a1.attendance_rate
        ),
        (Some(12), Some(10), Some(2), Some(0.833))
    );
    let b1 = cell("B", "1");
    assert_eq!((b1.assigned, b1.attendance_rate), (Some(12), Some(0.5)));
    let a5 = cell("A", "5");
    assert!(
        a5.suppressed && a5.assigned.is_none() && a5.attendance_rate.is_none(),
        "three is too few to show"
    );
    // No identifier anywhere.
    let json = serde_json::to_string(&r).unwrap();
    for (p, ap) in &ids {
        assert!(!json.contains(&p.to_string()) && !json.contains(&ap.to_string()));
    }
    assert!(s.experiments.readout(Uuid::new_v4(), None).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_start_list_read_out_and_stop() {
    let s = state().await;
    let (a, b) = (uniq("a"), uniq("b"));
    group(&s, &a, "A").await;
    group(&s, &b, "B").await;
    let app = patient_administration_system::api::rest::router(s);
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
    let name = uniq("msg");
    let (st, _) = call("POST", "/api/experiments".into(), Some(serde_json::json!({ "template_name": name, "variants": { "A": a }, "started_by": "Dr A" }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call("POST", "/api/experiments".into(), Some(serde_json::json!({ "template_name": name, "variants": { "A": a, "B": b }, "started_by": "Dr A" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let started: patient_administration_system::api::rest::handlers_experiments::Started =
        serde_json::from_value(v["data"].clone()).unwrap();
    let id = started.id.to_string();
    let (_, v) = call("GET", "/api/experiments".into(), None).await;
    let listed: Vec<patient_administration_system::experiments::Experiment> =
        serde_json::from_value(v["data"].clone()).unwrap();
    assert!(listed.iter().any(|e| e.id == started.id));
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == id.as_str())
    );
    let (st, v) = call("GET", format!("/api/experiments/{id}/readout"), None).await;
    let readout: patient_administration_system::experiments::Readout =
        serde_json::from_value(v["data"].clone()).unwrap();
    assert_eq!(readout.experiment_id, started.id);
    assert_eq!(
        (st, v["data"]["total_assigned"].as_i64()),
        (StatusCode::OK, Some(0))
    );
    let (st, _) = call(
        "POST",
        format!("/api/experiments/{id}/stop"),
        Some(serde_json::json!({ "stopped_by": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        "POST",
        format!("/api/experiments/{id}/stop"),
        Some(serde_json::json!({ "stopped_by": "Dr A" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        "GET",
        format!("/api/experiments/{}/readout", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}
