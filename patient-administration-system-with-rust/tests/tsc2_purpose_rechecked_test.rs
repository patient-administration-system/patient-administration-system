//! TSC-2: a message scheduled for later is checked again at the moment it would go. A reminder or a ladder
//! step for an appointment that was cancelled or moved after the scan found it is not sent, and the reason is
//! recorded. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test takes the appointment as the scan saw it, changes it in the database, and hands the stale copy to
//! the sender, which is the race the check closes: a scan reads many appointments, then sends one by one.

mod common;

use async_trait::async_trait;
use jiff::{SignedDuration, Timestamp};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::ladder::LadderService;
use patient_administration_system::communication::reminders::ReminderService;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::time::days;
use patient_administration_system::upstream::{Person, StubPersonClient};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

type Log = Arc<Mutex<Vec<String>>>;

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
        _b: &str,
    ) -> patient_administration_system::Result<()> {
        self.0.lock().unwrap().push(to.to_string());
        Ok(())
    }
}

struct World {
    s: AppState,
    appt: Appointment,
    phone: String,
    log: Log,
}

/// A booked appointment `ahead` from now, for a patient reachable by text.
async fn world(ahead: SignedDuration) -> World {
    // About when texts may go, not what: quiet hours would hold every text between 21:00 and 08:00.
    // SAFETY: tests that set environment variables run one at a time (`--test-threads=1`).
    unsafe { std::env::set_var("PAS_QUIET_HOURS", "off") };
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let person = Uuid::new_v4();
    let phone = format!("+447{:09}", Uuid::new_v4().as_u128() % 1_000_000_000);
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    data.insert(
        "telecom".into(),
        serde_json::json!([{ "system": "phone", "value": phone }]),
    );
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture(log.clone())));
    let start = Timestamp::now()
        + ahead
        + SignedDuration::from_mins((Uuid::new_v4().as_u128() % 10_000) as i64);
    let mut a = Appointment::new(person, start, start + SignedDuration::from_mins(30));
    a.status = AppointmentStatus::Booked;
    let appt = AppointmentRepository::create(&s.db, &a).await.unwrap();
    World {
        s,
        appt,
        phone,
        log,
    }
}

impl World {
    fn texts(&self) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|to| **to == self.phone)
            .count()
    }
    async fn cancel(&self) {
        self.s
            .db
            .run(Stmt::new(
                "UPDATE appointments SET status = 'cancelled', cancelled_at = now() WHERE id = $1",
                [self.appt.id.into()],
            ))
            .await
            .unwrap();
    }
    async fn move_by_an_hour(&self) {
        self.s
            .db
            .run(Stmt::new(
                "UPDATE appointments SET start_datetime = start_datetime + interval '1 hour', \
                 end_datetime = end_datetime + interval '1 hour' WHERE id = $1",
                [self.appt.id.into()],
            ))
            .await
            .unwrap();
    }
    async fn letters(&self) -> i64 {
        self.s
            .db
            .one(Stmt::new(
                "SELECT count(*)::bigint AS n FROM generated_letters WHERE appointment_id = $1",
                [self.appt.id.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .col("n")
            .unwrap()
    }
    async fn steps(&self) -> Vec<(String, String)> {
        let rows = self
            .s
            .db
            .all(Stmt::new(
                "SELECT step, outcome FROM reminder_steps WHERE appointment_id = $1 ORDER BY planned_for",
                [self.appt.id.into()],
            ))
            .await
            .unwrap();
        rows.iter()
            .map(|r| (r.col("step").unwrap(), r.col("outcome").unwrap()))
            .collect()
    }
    async fn call_tasks(&self) -> i64 {
        self.s
            .db
            .one(Stmt::new(
                "SELECT count(*)::bigint AS n FROM staff_call_tasks WHERE appointment_id = $1",
                [self.appt.id.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .col("n")
            .unwrap()
    }
    fn reminders(&self) -> ReminderService {
        ReminderService::new(self.s.db.clone(), self.s.communication.clone())
    }
    fn ladder(&self) -> LadderService {
        LadderService::new(self.s.db.clone(), self.s.communication.clone())
            .with_link_base(Some("https://pas.test".into()))
    }
}

/// Remind `stale` exactly as a scan would, with the default templates.
async fn remind(
    w: &World,
    stale: Appointment,
) -> patient_administration_system::communication::reminders::ReminderReport {
    let r = w.reminders();
    let templates = r.ensure_default_templates().await.unwrap();
    let channels = templates
        .first()
        .map(|t| t.channels.clone())
        .unwrap_or_default();
    r.remind(&templates, &channels, vec![stale], Timestamp::now())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_reminder_still_goes_when_nothing_changed() {
    let w = world(SignedDuration::from_hours(20)).await;
    let r = remind(&w, w.appt.clone()).await;
    assert_eq!((r.sent, r.skipped_no_longer_booked), (1, 0), "{r:?}");
    assert_eq!(w.texts(), 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_reminder_for_an_appointment_cancelled_after_the_scan_is_not_sent() {
    let w = world(SignedDuration::from_hours(20)).await;
    let stale = w.appt.clone();
    w.cancel().await;
    let r = remind(&w, stale).await;
    assert_eq!((r.sent, r.skipped_no_longer_booked), (0, 1), "{r:?}");
    assert_eq!(w.texts(), 0, "nothing went to the patient");
    assert_eq!(w.letters().await, 0, "no reminder letter was made");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_reminder_for_an_appointment_moved_after_the_scan_is_not_sent_with_the_old_time() {
    let w = world(SignedDuration::from_hours(20)).await;
    let stale = w.appt.clone();
    w.move_by_an_hour().await;
    let r = remind(&w, stale).await;
    assert_eq!((r.sent, r.skipped_no_longer_booked), (0, 1), "{r:?}");
    assert_eq!(w.texts(), 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_ladder_step_still_runs_when_nothing_changed() {
    let w = world(days(365)).await;
    let at = w.appt.start_datetime - days(7) + SignedDuration::from_mins(1);
    let r = w.ladder().run_for(vec![w.appt.clone()], at).await.unwrap();
    assert_eq!((r.sent, r.skipped_not_booked), (1, 0), "{r:?}");
    assert_eq!(w.steps().await, [("main".to_string(), "sent".to_string())]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_ladder_step_for_an_appointment_cancelled_after_the_scan_is_logged_and_not_sent() {
    let w = world(days(365)).await;
    let stale = w.appt.clone();
    w.cancel().await;
    let at = stale.start_datetime - days(7) + SignedDuration::from_mins(1);
    let r = w.ladder().run_for(vec![stale], at).await.unwrap();
    assert_eq!((r.sent, r.skipped_not_booked), (0, 1), "{r:?}");
    assert_eq!(
        w.steps().await,
        [("main".to_string(), "skipped_not_booked".to_string())],
        "the reason is recorded"
    );
    assert_eq!(w.texts(), 0);
    assert_eq!(w.letters().await, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_call_step_for_an_appointment_moved_after_the_scan_makes_no_call_task() {
    let w = world(days(365)).await;
    let stale = w.appt.clone();
    w.move_by_an_hour().await;
    // Just after T-3, the call step (main and second are logged as skipped late, unsent).
    let at = stale.start_datetime - days(3) + SignedDuration::from_mins(1);
    let r = w.ladder().run_for(vec![stale], at).await.unwrap();
    assert_eq!(r.skipped_not_booked, 1, "{r:?}");
    assert_eq!(
        w.call_tasks().await,
        0,
        "no person is asked to ring about a moved appointment"
    );
    assert_eq!(w.texts(), 0);
    assert!(
        w.steps()
            .await
            .iter()
            .any(|(_, o)| o == "skipped_not_booked"),
        "{:?}",
        w.steps().await
    );
}
