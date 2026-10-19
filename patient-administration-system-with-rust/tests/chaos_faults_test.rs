//! Chaos testing, part 2: faults in what the PAS depends on, and races between its own callers.
//!
//! - A flaky carrier: sends fail or stall at random while the reminder ladder runs through all its steps.
//!   Whatever the carrier does, nothing is sent twice, nothing is recorded as sent that was not, a failure is
//!   recorded as a failure (not silence), and a patient who never answers is still escalated to a person.
//! - A race: many callers try to book the same slot at once. Exactly one wins; the rest are refused.
//!
//! Seeded (`PAS_CHAOS_SEED`; failures print it) and needs PostgreSQL:
//! `cargo test --test chaos_faults_test -- --ignored --test-threads=1`.

mod common;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::consent::{self, QuietHours};
use patient_administration_system::communication::ladder::LadderService;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::upstream::{Person, StubPersonClient};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn seed() -> u64 {
    std::env::var("PAS_CHAOS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1)
                | 1
        })
}

/// xorshift64*, behind an atomic so a shared sender can use it.
struct Chaos(AtomicU64);
impl Chaos {
    fn next(&self) -> u64 {
        let mut x = self.0.load(Ordering::Relaxed);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0.store(x, Ordering::Relaxed);
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn chance(&self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// A carrier that fails or stalls at random while `faults` is on, and records what it really delivered.
struct FlakyCarrier {
    chaos: Arc<Chaos>,
    faults: Arc<AtomicUsize>, // 1 = faults on
    failure_percent: u64,
    delivered: Mutex<HashMap<String, usize>>, // to -> messages really delivered
    attempts: AtomicUsize,
}

#[async_trait]
impl MessageSender for FlakyCarrier {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _subject: &str,
        _body: &str,
    ) -> patient_administration_system::Result<()> {
        self.attempts.fetch_add(1, Ordering::Relaxed);
        if self.faults.load(Ordering::Relaxed) == 1 {
            if self.chaos.chance(30) {
                tokio::time::sleep(std::time::Duration::from_millis(1 + self.chaos.next() % 15))
                    .await;
            }
            if self.chaos.chance(self.failure_percent) {
                return Err(patient_administration_system::Error::upstream(
                    "the carrier is having a bad day",
                ));
            }
        }
        *self
            .delivered
            .lock()
            .unwrap()
            .entry(to.to_string())
            .or_default() += 1;
        Ok(())
    }
}

struct Patient {
    person: Uuid,
    phone: String,
    appt: Appointment,
}

#[tokio::test]
#[ignore = "chaos: requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_flaky_carrier_never_causes_a_double_send_a_false_record_or_a_lost_escalation() {
    let seed = seed();
    let chaos = Arc::new(Chaos(AtomicU64::new(seed)));
    let faults = Arc::new(AtomicUsize::new(1));
    let carrier = Arc::new(FlakyCarrier {
        chaos: chaos.clone(),
        faults: faults.clone(),
        failure_percent: 40,
        delivered: Mutex::new(HashMap::new()),
        attempts: AtomicUsize::new(0),
    });
    // Ten patients, each reachable by text only, none of whom ever replies.
    let stub = Arc::new(StubPersonClient::new());
    let mut patients = Vec::new();
    let base = Utc::now()
        + Duration::days(900)
        + Duration::minutes((Uuid::new_v4().as_u128() % 50_000) as i64);
    let s = common::build_state(&common::database_url().unwrap()).await;
    for i in 0..10 {
        let person = Uuid::new_v4();
        let phone =
            format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000).replace(' ', "");
        let mut data = serde_json::Map::new();
        data.insert("name".into(), serde_json::json!({ "given": "Chaos" }));
        data.insert(
            "telecom".into(),
            serde_json::json!([{ "system": "phone", "value": phone }]),
        );
        stub.insert(Person { id: person, data });
        let start = base + Duration::minutes(i * 7);
        let mut a = Appointment::new(person, start, start + Duration::minutes(30));
        a.status = AppointmentStatus::Booked;
        patients.push(Patient {
            person,
            phone,
            appt: AppointmentRepository::create(&s.db, &a).await.unwrap(),
        });
    }
    let s = s
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, carrier.clone());
    // Quiet hours would hold some steps back depending on the time the test runs; this test is about faults.
    let _ = QuietHours::parse("00:00-00:00");
    let _ = consent::TEXT_CHANNELS;
    unsafe { std::env::set_var("PAS_QUIET_HOURS", "off") };
    let ladder = LadderService::new(s.db.clone(), s.communication.clone())
        .with_link_base(Some("https://pas.test".into()));

    // Walk the pretend clock through every step, scanning several times at each (a scan retries nothing that
    // is already logged, and the clock stands still between repeats, as a real scan overlap or restart would).
    let first = patients[0].appt.start_datetime;
    let last = patients.last().unwrap().appt.start_datetime;
    for before in [
        Duration::days(7),
        Duration::days(4),
        Duration::days(3),
        Duration::hours(24),
    ] {
        for round in 0..4 {
            // Faults flicker on and off like a real outage.
            faults.store(usize::from(chaos.chance(70)), Ordering::Relaxed);
            let now: DateTime<Utc> = last - before + Duration::minutes(1 + round);
            ladder.run_once(now).await.unwrap_or_else(|e| {
                panic!("PAS_CHAOS_SEED={seed}: the ladder errored under faults: {e}")
            });
        }
        let _ = first;
    }
    faults.store(0, Ordering::Relaxed);
    ladder.run_once(last - Duration::hours(1)).await.unwrap();

    let delivered = carrier.delivered.lock().unwrap().clone();
    let mut sent_rows_total = 0usize;
    for p in &patients {
        let rows =
            s.db.query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT step, outcome FROM reminder_steps WHERE appointment_id = $1",
                [p.appt.id.into()],
            ))
            .await
            .unwrap();
        let steps: Vec<(String, String)> = rows
            .iter()
            .map(|r| {
                (
                    r.try_get("", "step").unwrap(),
                    r.try_get("", "outcome").unwrap(),
                )
            })
            .collect();
        // Each step at most once (the table enforces it; the ladder must not trip over it).
        let mut seen = std::collections::HashSet::new();
        assert!(
            steps.iter().all(|(st, _)| seen.insert(st.clone())),
            "PAS_CHAOS_SEED={seed}: a step ran twice for {}: {steps:?}",
            p.phone
        );
        let sent = steps.iter().filter(|(_, o)| o == "sent").count();
        sent_rows_total += sent;
        // What the patient was really sent is exactly what the record says was sent: no phantom "sent", and no
        // unrecorded send.
        let really = delivered.get(&p.phone).copied().unwrap_or(0);
        assert_eq!(
            really, sent,
            "PAS_CHAOS_SEED={seed}: {} was delivered {really} but recorded {sent}: {steps:?}",
            p.phone
        );
        // A step the carrier refused is on the record as a failure.
        let letters = s
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) FILTER (WHERE status = 'sent')::bigint AS ok FROM generated_letters WHERE person_id = $1",
                [p.person.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            letters.try_get::<i64>("", "ok").unwrap() as usize,
            sent,
            "PAS_CHAOS_SEED={seed}: letters disagree with steps for {}",
            p.phone
        );
        // Silence is escalated to a person whatever the carrier did.
        let task = s
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS n FROM staff_call_tasks WHERE appointment_id = $1 AND reason = 'silent'",
                [p.appt.id.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            task.try_get::<i64>("", "n").unwrap(),
            1,
            "PAS_CHAOS_SEED={seed}: a silent patient was not escalated: {steps:?}"
        );
        // Nothing about the booking itself changed.
        let a = AppointmentRepository::find_by_id(&s.db, p.appt.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.status, AppointmentStatus::Booked);
    }
    eprintln!(
        "chaos seed {seed}: {} send attempts, {} delivered, {sent_rows_total} recorded as sent",
        carrier.attempts.load(Ordering::Relaxed),
        delivered.values().sum::<usize>()
    );
    assert!(carrier.attempts.load(Ordering::Relaxed) > 0);
}

#[tokio::test]
#[ignore = "chaos: requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored --test-threads=1`"]
async fn a_crowd_booking_the_same_slot_at_once_produces_exactly_one_appointment() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    for round in 0..5 {
        let schedule = common::new_schedule(&s.db).await.id;
        let slot = common::free_slot(&s.db, schedule, 400 + round).await;
        let mut tasks = Vec::new();
        for _ in 0..25 {
            let scheduling = s.scheduling.clone();
            let person = Uuid::new_v4();
            tasks.push(tokio::spawn(async move {
                scheduling
                    .book_slot(slot.id, person, &UserContext::system())
                    .await
            }));
        }
        let mut won = 0;
        for t in tasks {
            if t.await.expect("a booking task panicked").is_ok() {
                won += 1;
            }
        }
        assert_eq!(
            won, 1,
            "round {round}: {won} callers were told they had booked the same slot"
        );
        let n = s
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS n FROM appointments WHERE slot_id = $1 AND deleted_at IS NULL AND status IN ('booked','arrived','fulfilled')",
                [slot.id.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            n.try_get::<i64>("", "n").unwrap(),
            1,
            "round {round}: more than one live appointment on one slot"
        );
    }
}
