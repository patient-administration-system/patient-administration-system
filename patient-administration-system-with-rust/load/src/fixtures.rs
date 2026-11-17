//! The rows a run needs, made through the server's own repositories (the API has no route that creates a
//! schedule or a slot), and the checks read back from the database after a run.
//!
//! Every schedule a run makes has the service type `load-<run id>-<purpose>`, and every slot starts at
//! least 600 days ahead, so a run's rows never meet a person's real ones or another run's. Nothing is
//! deleted afterwards: the rows are the evidence of what the run did.

use jiff::{SignedDuration, Timestamp};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::connect;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::schedule::ScheduleRepository;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::schedule::{Schedule, ScheduleOwner, Slot};
use patient_administration_system::streaming::InMemoryEventPublisher;
use std::sync::Arc;
use uuid::Uuid;

pub type Fail = Box<dyn std::error::Error + Send + Sync>;

/// How many clinicians the cycle scenario's slots are spread over.
const CYCLE_OWNERS: usize = 50;

/// How many of each kind of row to make.
#[derive(Debug, Clone, Copy)]
pub struct Sizes {
    /// Slots for reading, half of them booked so there are appointments to read.
    pub read: usize,
    /// Free slots for the book, read, cancel cycle, over 50 clinicians; a cancelled slot is free again, so
    /// they are reused.
    pub cycle: usize,
    /// The few slots every worker races for in the contention scenario.
    pub hot: usize,
    /// Free slots behind one practitioner for the IHE `$find`, `$hold`, `$book` flow.
    pub ihe: usize,
}

#[derive(Debug, Clone)]
pub struct Fixtures {
    pub read_slots: Vec<Uuid>,
    pub read_appointments: Vec<Uuid>,
    /// The day the read appointments start on, for the calendar range query.
    pub read_day: Timestamp,
    pub cycle_slots: Vec<Uuid>,
    pub hot_slots: Vec<Uuid>,
    pub ihe_practitioner: Uuid,
    pub ihe_window: (Timestamp, Timestamp),
}

pub struct Db {
    state: AppState,
}

impl Db {
    pub async fn open(url: &str) -> Result<Self, Fail> {
        let db = connect(url).await?;
        Ok(Self {
            state: AppState::new(db, Arc::new(InMemoryEventPublisher::new())),
        })
    }

    async fn schedule(&self, run: &str, purpose: &str, owner: Uuid) -> Result<Schedule, Fail> {
        let s = Schedule::new(
            ScheduleOwner::Practitioner(owner),
            format!("load-{run}-{purpose}"),
        );
        Ok(ScheduleRepository::create(&self.state.db, &s).await?)
    }

    /// `n` back-to-back 15-minute slots from `start`.
    async fn slots(&self, schedule: Uuid, start: Timestamp, n: usize) -> Result<Vec<Slot>, Fail> {
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let at = start + SignedDuration::from_mins(15 * i64::try_from(i)?);
            let slot = Slot::new(schedule, at, at + SignedDuration::from_mins(15));
            out.push(SlotRepository::create(&self.state.db, &slot).await?);
        }
        Ok(out)
    }

    pub async fn make(&self, run_id: &str, sizes: Sizes) -> Result<Fixtures, Fail> {
        // A start far ahead and different for each run, on a whole hour, so the read day is predictable.
        let offset = i64::from(u32::from_str_radix(&run_id[..4], 16).unwrap_or(0) % 2000);
        let base = (Timestamp::now() + patient_administration_system::time::days(600 + offset))
            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
            .date()
            .at(8, 0, 0, 0)
            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
            .map(|z| z.timestamp())
            .map_err(|_| "no 08:00 on the base day")?;

        let read = self.schedule(run_id, "read", Uuid::new_v4()).await?;
        let read_slots = self.slots(read.id, base, sizes.read).await?;
        let mut read_appointments = Vec::new();
        for s in read_slots.iter().step_by(2) {
            let a = self
                .state
                .scheduling
                .book_slot(s.id, Uuid::new_v4(), &UserContext::system())
                .await?;
            read_appointments.push(a.id);
        }

        // Spread over many clinicians: a booking locks its schedule's owner (CORE-3), so slots that all
        // belonged to one clinician would measure that one lock, not the server.
        let mut cycle_slots = Vec::with_capacity(sizes.cycle);
        let per = sizes.cycle.div_ceil(CYCLE_OWNERS);
        for _ in 0..CYCLE_OWNERS {
            let cycle = self.schedule(run_id, "cycle", Uuid::new_v4()).await?;
            cycle_slots.extend(
                self.slots(
                    cycle.id,
                    base + patient_administration_system::time::days(30),
                    per,
                )
                .await?,
            );
        }
        let hot = self.schedule(run_id, "hot", Uuid::new_v4()).await?;
        let hot_slots = self
            .slots(
                hot.id,
                base + patient_administration_system::time::days(60),
                sizes.hot,
            )
            .await?;
        let practitioner = Uuid::new_v4();
        let ihe = self.schedule(run_id, "ihe", practitioner).await?;
        let ihe_start = base + patient_administration_system::time::days(90);
        let ihe_slots = self.slots(ihe.id, ihe_start, sizes.ihe).await?;
        let ihe_end = ihe_slots
            .last()
            .map_or(ihe_start, |s| s.end_datetime + SignedDuration::from_mins(1));

        Ok(Fixtures {
            read_slots: read_slots.iter().map(|s| s.id).collect(),
            read_appointments,
            read_day: base,
            cycle_slots: cycle_slots.iter().map(|s| s.id).collect(),
            hot_slots: hot_slots.iter().map(|s| s.id).collect(),
            ihe_practitioner: practitioner,
            ihe_window: (ihe_start, ihe_end),
        })
    }

    /// For each hot slot, how many appointments hold it now (booked or arrived). More than one is a
    /// double booking.
    pub async fn holders_per_slot(&self, slots: &[Uuid]) -> Result<Vec<(Uuid, i64)>, Fail> {
        let mut out = Vec::with_capacity(slots.len());
        for id in slots {
            let row = self
                .state
                .db
                .one(Stmt::new(
                    "SELECT count(*) AS n FROM appointments WHERE slot_id = $1 \
                     AND status IN ('booked', 'arrived') AND deleted_at IS NULL",
                    [(*id).into()],
                ))
                .await?
                .ok_or("no count row")?;
            out.push((*id, row.col::<i64>("n")?));
        }
        Ok(out)
    }

    /// How many of these appointments have a `book` audit row, and how many `AppointmentBooked` outbox
    /// events name them (NFR-1, NFR-2).
    pub async fn audit_and_outbox(&self, appointments: &[Uuid]) -> Result<(i64, i64), Fail> {
        let ids = appointments
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let audit = self
            .state
            .db
            .one(Stmt::new(
                "SELECT count(DISTINCT entity_id) AS n FROM audit_log WHERE action = 'book' \
                 AND entity_type = 'appointment' AND entity_id::text = ANY(string_to_array($1, ','))",
                [ids.clone().into()],
            ))
            .await?
            .ok_or("no count row")?
            .col::<i64>("n")?;
        let outbox = self
            .state
            .db
            .one(Stmt::new(
                "SELECT count(DISTINCT payload->>'appointment_id') AS n FROM outbox_events \
                 WHERE event_type = 'AppointmentBooked' \
                 AND payload->>'appointment_id' = ANY(string_to_array($1, ','))",
                [ids.into()],
            ))
            .await?
            .ok_or("no count row")?
            .col::<i64>("n")?;
        Ok((audit, outbox))
    }
}
