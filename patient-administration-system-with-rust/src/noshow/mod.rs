//! The no-show probability (CAP-12, CAS-5): support only, interpretable, and gated by people.
//!
//! **What it is.** A transparent weighted sum, not a trained black box: a base rate, plus the patient's own
//! history of missed visits (shrunk towards the base rate when there are few past visits, so one miss out of
//! two is not read as 50% evidence), plus small adjustments for lead time and age band, and optional ones for
//! day of week and time of day (zero by default: they are the factors most open to steering unreliable slots
//! to particular groups). The weights are configuration (`PAS_NOSHOW_WEIGHTS`), so a clinician can read and
//! challenge them. The factors behind each number are stored and shown.
//!
//! **What it may do (NFR-11).** It may add support: put an appointment on the staff priority list so a person
//! rings the patient. It never removes, reorders away or withholds a slot, never overbooks, never delays a
//! contact, and acts on nothing by itself (UK GDPR Art. 22: staff decide). Nothing in the booking, rebooking,
//! standby or offer code reads it, a test proves they behave identically without it, and the schema guard
//! (`cap10_support_test`) lists its columns by name.
//!
//! **The gate.** It is off. It is on only when `PAS_NOSHOW_SCORE=true` *and* a named person has approved a
//! fairness review *and* a named person has approved a clinical safety review of this model version, and
//! nobody has since rejected either (a rejection switches it off at once). The fairness audit
//! ([`NoShowService::fairness_audit`]) backtests the model on past appointments by deprivation quintile and
//! language, with small cells withheld, so the reviewer has evidence. A risk score also re-opens the medical
//! device question and the clinical safety case (DTAC-1): a person must decide those, and this code cannot.
//!
//! **Not built:** distance from the clinic (needs a postcode-district band no one supplies yet), clinic type
//! as a factor, and weather or travel forecasts.

use chrono::{DateTime, Datelike, Timelike, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::analytics::{AnalyticsService, suppress};
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

pub const MODEL_VERSION: &str = "weighted-sum-1";
/// A fairness review stays valid this long; clinical safety, a year.
const FAIRNESS_VALID_DAYS: i64 = 180;
const SAFETY_VALID_DAYS: i64 = 365;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    Low,
    Medium,
    High,
}

/// Low up to the medium threshold, medium above it, high above the high one (`> 0.70` is high, `0.70` is not).
pub fn band(p: f64, medium: f64, high: f64) -> Band {
    if p > high {
        Band::High
    } else if p > medium {
        Band::Medium
    } else {
        Band::Low
    }
}

pub fn thresholds() -> (f64, f64) {
    let get = |k: &str, d: f64| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| (0.0..=1.0).contains(v))
            .unwrap_or(d)
    };
    let (m, h) = (get("PAS_NOSHOW_MEDIUM", 0.40), get("PAS_NOSHOW_HIGH", 0.70));
    if m < h { (m, h) } else { (0.40, 0.70) }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Weights {
    /// The no-show rate assumed with no information.
    pub base: f64,
    /// How much the patient's own (shrunk) miss rate moves the probability, 0 to 1.
    pub history: f64,
    /// How many "average" visits the base rate is worth when shrinking a small sample.
    pub prior_visits: f64,
    /// Added at a lead time of 90 days or more (scaled linearly below it).
    pub lead: f64,
    /// Added by age band. Bands may only raise the probability, so support goes where it is needed.
    pub age: BTreeMap<String, f64>,
    /// Added by day of week (Monday first) and by part of day (morning, afternoon, evening). Zero by default.
    pub weekday: [f64; 7],
    pub period: [f64; 3],
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            base: 0.10,
            history: 0.9,
            prior_visits: 3.0,
            lead: 0.06,
            age: [("16-24", 0.03), ("25-34", 0.02)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            weekday: [0.0; 7],
            period: [0.0; 3],
        }
    }
}

impl Weights {
    /// `PAS_NOSHOW_WEIGHTS` as JSON overriding any of the defaults; an unreadable value keeps the defaults.
    pub fn from_env() -> Self {
        std::env::var("PAS_NOSHOW_WEIGHTS")
            .ok()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct Inputs {
    /// Past completed appointments the patient missed, and attended.
    pub past_missed: i64,
    pub past_attended: i64,
    pub lead_days: f64,
    pub age_band: Option<String>,
    /// Monday is 0. `hour` is the clinic-local hour.
    pub weekday: usize,
    pub hour: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Factor {
    pub name: String,
    /// What was measured, in words a front-desk member can read.
    pub detail: String,
    /// How far it moved the probability, in percentage points (can be zero).
    pub effect_points: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Assessment {
    pub probability: f64,
    pub factors: Vec<Factor>,
    /// Past completed appointments the history is based on.
    pub sample_size: i64,
}

/// The model: a pure function of the inputs and the weights.
pub fn compute(i: &Inputs, w: &Weights) -> Assessment {
    let n = i.past_missed + i.past_attended;
    let shrunk = (i.past_missed as f64 + w.prior_visits * w.base) / (n as f64 + w.prior_visits);
    let history = w.history * (shrunk - w.base);
    let lead = w.lead * (i.lead_days.clamp(0.0, 90.0) / 90.0);
    let age = i
        .age_band
        .as_ref()
        .and_then(|b| w.age.get(b))
        .copied()
        .unwrap_or(0.0);
    let weekday = w.weekday[i.weekday.min(6)];
    let period = w.period[match i.hour {
        0..=11 => 0,
        12..=16 => 1,
        _ => 2,
    }];
    let points = |x: f64| (x * 1000.0).round() / 10.0;
    let probability = (w.base + history + lead + age + weekday + period).clamp(0.01, 0.95);
    let factors = vec![
        Factor {
            name: "base_rate".into(),
            detail: format!(
                "{:.0}% is the rate assumed with no information",
                w.base * 100.0
            ),
            effect_points: points(w.base),
        },
        Factor {
            name: "past_visits".into(),
            detail: if n == 0 {
                "no completed past visits to go on".into()
            } else {
                format!("missed {} of {} completed past visits", i.past_missed, n)
            },
            effect_points: points(history),
        },
        Factor {
            name: "lead_time".into(),
            detail: format!("booked {:.0} days ahead", i.lead_days.max(0.0)),
            effect_points: points(lead),
        },
        Factor {
            name: "age_band".into(),
            detail: i.age_band.clone().unwrap_or_else(|| "not known".into()),
            effect_points: points(age),
        },
        Factor {
            name: "day_and_time".into(),
            detail: "day of week and part of day".into(),
            effect_points: points(weekday + period),
        },
    ];
    Assessment {
        probability: (probability * 10000.0).round() / 10000.0,
        factors,
        sample_size: n,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub model_version: &'static str,
    pub switched_on: bool,
    pub fairness_approved: bool,
    pub clinical_safety_approved: bool,
    pub active: bool,
    /// Why it is off, in words, when it is.
    pub reason: Option<String>,
    pub medium_threshold: f64,
    pub high_threshold: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Shown {
    pub appointment_id: Uuid,
    pub probability_percent: f64,
    pub band: Band,
    pub factors: Vec<Factor>,
    pub sample_size: i64,
    pub model_version: String,
    pub computed_at: DateTime<Utc>,
}

pub struct NoShowService {
    db: DatabaseConnection,
}

impl NoShowService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// The cheap check before any work: is the switch on? (It is not enough on its own; see [`Self::status`].)
    pub fn switched_on() -> bool {
        std::env::var("PAS_NOSHOW_SCORE").as_deref() == Ok("true")
    }

    async fn latest_review(&self, kind: &str, valid_days: i64) -> Result<Option<String>> {
        Ok(self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT outcome, reviewed_at > now() - make_interval(days => $3::int) AS fresh FROM noshow_reviews \
                 WHERE kind = $1 AND model_version = $2 ORDER BY reviewed_at DESC LIMIT 1",
                [kind.into(), MODEL_VERSION.into(), (valid_days as i32).into()],
            ))
            .await
            .map_err(Error::Database)?
            .and_then(|r| {
                let outcome: String = r.try_get("", "outcome").ok()?;
                let fresh: bool = r.try_get("", "fresh").ok()?;
                (outcome == "approved" && fresh).then_some(outcome)
            }))
    }

    pub async fn status(&self) -> Result<Status> {
        let switched_on = Self::switched_on();
        let fairness = self
            .latest_review("fairness", FAIRNESS_VALID_DAYS)
            .await?
            .is_some();
        let safety = self
            .latest_review("clinical_safety", SAFETY_VALID_DAYS)
            .await?
            .is_some();
        let active = switched_on && fairness && safety;
        let reason = (!active).then(|| {
            let mut missing = Vec::new();
            if !switched_on {
                missing.push("PAS_NOSHOW_SCORE is not true".to_string());
            }
            if !fairness {
                missing.push(format!(
                    "no current approved fairness review (valid {FAIRNESS_VALID_DAYS} days)"
                ));
            }
            if !safety {
                missing.push(format!(
                    "no current approved clinical safety review (valid {SAFETY_VALID_DAYS} days)"
                ));
            }
            missing.join("; ")
        });
        let (medium_threshold, high_threshold) = thresholds();
        Ok(Status {
            model_version: MODEL_VERSION,
            switched_on,
            fairness_approved: fairness,
            clinical_safety_approved: safety,
            active,
            reason,
            medium_threshold,
            high_threshold,
        })
    }

    async fn require_active(&self) -> Result<()> {
        let s = self.status().await?;
        if s.active {
            Ok(())
        } else {
            Err(Error::conflict(format!(
                "the no-show probability is switched off: {}",
                s.reason.unwrap_or_default()
            )))
        }
    }

    /// Record a person's review of this model version. An approval needs notes saying what was looked at.
    pub async fn record_review(
        &self,
        kind: &str,
        reviewer: &str,
        outcome: &str,
        notes: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        if !["fairness", "clinical_safety"].contains(&kind) {
            return Err(Error::validation(
                "kind must be fairness or clinical_safety",
            ));
        }
        if !["approved", "rejected"].contains(&outcome) {
            return Err(Error::validation("outcome must be approved or rejected"));
        }
        let (reviewer, notes) = (reviewer.trim(), notes.trim());
        if reviewer.is_empty() {
            return Err(Error::validation(
                "a review is made by a named person (reviewer)",
            ));
        }
        if outcome == "approved" && notes.chars().count() < 40 {
            return Err(Error::validation(
                "an approval needs notes (at least 40 characters) saying what was looked at and why it is acceptable",
            ));
        }
        if notes.is_empty() || notes.chars().count() > 1000 {
            return Err(Error::validation("notes must be 1 to 1000 characters"));
        }
        let id = Uuid::new_v4();
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO noshow_reviews (id, kind, reviewer, outcome, notes, model_version) VALUES ($1,$2,$3,$4,$5,$6)",
                [id.into(), kind.into(), reviewer.into(), outcome.into(), notes.into(), MODEL_VERSION.into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log_with(
            &self.db,
            "noshow_model",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, MODEL_VERSION.as_bytes()),
            &format!("review_{outcome}"),
            None,
            Some(serde_json::json!({ "kind": kind, "reviewer": reviewer })),
            Some(kind),
            None,
            ctx,
        )
        .await
    }

    async fn inputs_for(
        &self,
        appointment_id: Uuid,
    ) -> Result<Option<(Uuid, Inputs, DateTime<Utc>)>> {
        let tz = crate::inbound::messages::timezone();
        let Some(r) = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT a.person_id, a.start_datetime, a.created_at, a.updated_at, d.age_band, \
                   (SELECT count(*) FROM appointments p WHERE p.person_id = a.person_id AND p.id <> a.id \
                      AND p.deleted_at IS NULL AND p.start_datetime < now() AND p.status = 'no_show')::bigint AS missed, \
                   (SELECT count(*) FROM appointments p WHERE p.person_id = a.person_id AND p.id <> a.id \
                      AND p.deleted_at IS NULL AND p.start_datetime < now() AND p.status IN ('fulfilled','arrived'))::bigint AS attended \
                 FROM appointments a LEFT JOIN person_demographics d ON d.person_id = a.person_id \
                 WHERE a.id = $1 AND a.deleted_at IS NULL",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?
        else {
            return Ok(None);
        };
        let g = Error::Database;
        let start: DateTime<Utc> = r.try_get("", "start_datetime").map_err(g)?;
        let created: DateTime<Utc> = r.try_get("", "created_at").map_err(g)?;
        let updated: DateTime<Utc> = r.try_get("", "updated_at").map_err(g)?;
        let local = start.with_timezone(&tz);
        Ok(Some((
            r.try_get("", "person_id").map_err(g)?,
            Inputs {
                past_missed: r.try_get("", "missed").map_err(g)?,
                past_attended: r.try_get("", "attended").map_err(g)?,
                lead_days: ((start - created).num_seconds() as f64 / 86_400.0).max(0.0),
                age_band: r.try_get("", "age_band").map_err(g)?,
                weekday: local.weekday().num_days_from_monday() as usize,
                hour: local.hour(),
            },
            updated,
        )))
    }

    /// Score one booked appointment, once a day (a rerun the same day changes nothing unless the appointment
    /// changed), and put it on the staff priority list if it is above the high threshold.
    pub async fn assess(&self, appointment_id: Uuid) -> Result<Shown> {
        self.require_active().await?;
        let now = Utc::now();
        if let Some(existing) = self.stored(appointment_id).await? {
            let row = self
                .db
                .query_one(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT (n.computed_on = $2 AND a.updated_at <= n.computed_at) AS current \
                     FROM noshow_assessments n JOIN appointments a ON a.id = n.appointment_id WHERE n.appointment_id = $1",
                    [appointment_id.into(), now.date_naive().into()],
                ))
                .await
                .map_err(Error::Database)?;
            if row.and_then(|r| r.try_get::<bool>("", "current").ok()) == Some(true) {
                return Ok(existing);
            }
        }
        let (person, inputs, _) = self
            .inputs_for(appointment_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("appointment {appointment_id}")))?;
        let a = compute(&inputs, &Weights::from_env());
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO noshow_assessments (appointment_id, probability, factors, sample_size, model_version, computed_at, computed_on) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (appointment_id) DO UPDATE SET probability = $2, factors = $3, \
                 sample_size = $4, model_version = $5, computed_at = $6, computed_on = $7",
                [
                    appointment_id.into(),
                    a.probability.into(),
                    serde_json::to_value(&a.factors).unwrap_or_default().into(),
                    (a.sample_size as i32).into(),
                    MODEL_VERSION.into(),
                    now.into(),
                    now.date_naive().into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        let (_, high) = thresholds();
        if a.probability > high {
            // Support only: a task for a person to ring. It never touches the booking.
            self.db
                .execute(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) \
                     SELECT $1, a.id, a.person_id, 'high_risk', a.start_datetime - interval '2 days' FROM appointments a \
                     WHERE a.id = $2 AND a.status = 'booked' ON CONFLICT (appointment_id, reason) DO NOTHING",
                    [Uuid::new_v4().into(), appointment_id.into()],
                ))
                .await
                .map_err(Error::Database)?;
        }
        let _ = person;
        self.stored(appointment_id)
            .await?
            .ok_or_else(|| Error::internal("assessment vanished"))
    }

    async fn stored(&self, appointment_id: Uuid) -> Result<Option<Shown>> {
        let (medium, high) = thresholds();
        self.db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT probability, factors, sample_size, model_version, computed_at FROM noshow_assessments WHERE appointment_id = $1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .map(|r| {
                let p: f64 = r.try_get("", "probability").map_err(Error::Database)?;
                let factors: serde_json::Value = r.try_get("", "factors").map_err(Error::Database)?;
                Ok(Shown {
                    appointment_id,
                    probability_percent: (p * 1000.0).round() / 10.0,
                    band: band(p, medium, high),
                    factors: serde_json::from_value(factors).unwrap_or_default(),
                    sample_size: i64::from(r.try_get::<i32>("", "sample_size").map_err(Error::Database)?),
                    model_version: r.try_get("", "model_version").map_err(Error::Database)?,
                    computed_at: r.try_get("", "computed_at").map_err(Error::Database)?,
                })
            })
            .transpose()
    }

    /// The stored assessments for these appointments, for a page that shows many at once. Read-only (it
    /// scores nothing, so a page load never does work); empty while the score is off or for appointments the
    /// daily pass has not reached.
    pub async fn for_appointments(&self, ids: &[Uuid]) -> Result<(bool, Vec<Shown>)> {
        if !self.status().await?.active || ids.is_empty() {
            return Ok((false, Vec::new()));
        }
        let mut out = Vec::new();
        for id in ids.iter().take(500) {
            if let Some(s) = self.stored(*id).await? {
                out.push(s);
            }
        }
        Ok((true, out))
    }

    /// The assessment of a booked appointment, made now if it has none or is out of date.
    pub async fn for_appointment(&self, appointment_id: Uuid) -> Result<Shown> {
        self.assess(appointment_id).await
    }

    /// The daily job: assess every booked appointment still to come. Idempotent within a day.
    pub async fn recalculate_all(&self) -> Result<usize> {
        self.require_active().await?;
        let ids = self
            .db
            .query_all(Statement::from_string(
                DbBackend::Postgres,
                "SELECT id FROM appointments WHERE status = 'booked' AND deleted_at IS NULL AND start_datetime > now() \
                 ORDER BY start_datetime LIMIT 20000"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let mut n = 0;
        for r in ids {
            let id: Uuid = r.try_get("", "id").map_err(Error::Database)?;
            self.assess(id).await?;
            n += 1;
        }
        Ok(n)
    }

    /// Booked appointments above the high threshold that nobody has dealt with, soonest first, with the factors.
    pub async fn priority_list(&self, limit: i64) -> Result<Vec<serde_json::Value>> {
        self.require_active().await?;
        let (_, high) = thresholds();
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT n.appointment_id, n.probability, n.factors, n.sample_size, a.start_datetime, t.id AS task_id \
                 FROM noshow_assessments n JOIN appointments a ON a.id = n.appointment_id \
                 JOIN staff_call_tasks t ON t.appointment_id = n.appointment_id AND t.reason = 'high_risk' \
                 WHERE a.status = 'booked' AND a.start_datetime > now() AND t.handled_at IS NULL AND n.probability > $1 \
                 ORDER BY a.start_datetime LIMIT $2",
                [high.into(), limit.clamp(1, 500).into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                let p: f64 = r.try_get("", "probability").map_err(Error::Database)?;
                Ok(serde_json::json!({
                    "appointment_id": r.try_get::<Uuid>("", "appointment_id").map_err(Error::Database)?,
                    "task_id": r.try_get::<Uuid>("", "task_id").map_err(Error::Database)?,
                    "probability_percent": (p * 1000.0).round() / 10.0,
                    "starts_at": r.try_get::<DateTime<Utc>>("", "start_datetime").map_err(Error::Database)?,
                    "sample_size": r.try_get::<i32>("", "sample_size").map_err(Error::Database)?,
                    "factors": r.try_get::<serde_json::Value>("", "factors").map_err(Error::Database)?,
                }))
            })
            .collect()
    }

    /// Backtest the model on appointments that have already had an outcome, as the model would have scored
    /// them with only what was known then, split by deprivation quintile and language. For a reviewer: it
    /// shows whether the model's predictions fall harder on one group than the outcomes do. Cells under k are
    /// withheld. This runs whether or not the score is switched on: evidence comes before approval.
    pub async fn fairness_audit(&self, days: i64, k: Option<i64>) -> Result<serde_json::Value> {
        let k = k.unwrap_or(0).max(AnalyticsService::minimum_k());
        let tz = crate::inbound::messages::timezone();
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT a.status, a.start_datetime, a.created_at, d.age_band, \
                   COALESCE(d.deprivation_quintile::text, 'unknown') AS q, COALESCE(cp.preferred_language, 'unknown') AS lang, \
                   (SELECT count(*) FROM appointments p WHERE p.person_id = a.person_id AND p.id <> a.id AND p.deleted_at IS NULL \
                      AND p.start_datetime < a.start_datetime AND p.status = 'no_show')::bigint AS missed, \
                   (SELECT count(*) FROM appointments p WHERE p.person_id = a.person_id AND p.id <> a.id AND p.deleted_at IS NULL \
                      AND p.start_datetime < a.start_datetime AND p.status IN ('fulfilled','arrived'))::bigint AS attended \
                 FROM appointments a LEFT JOIN person_demographics d ON d.person_id = a.person_id \
                 LEFT JOIN patient_communication_profiles cp ON cp.person_id = a.person_id \
                 WHERE a.deleted_at IS NULL AND a.status IN ('fulfilled','arrived','no_show') \
                   AND a.start_datetime > now() - make_interval(days => $1::int) AND a.start_datetime < now() \
                 ORDER BY a.start_datetime DESC LIMIT 50000",
                [(days.clamp(1, 1825) as i32).into()],
            ))
            .await
            .map_err(Error::Database)?;
        let (_, high) = thresholds();
        let w = Weights::from_env();
        #[derive(Default)]
        struct Acc {
            n: i64,
            predicted: f64,
            high: i64,
            missed: i64,
        }
        let mut cells: BTreeMap<(String, String), Acc> = BTreeMap::new();
        let mut all = Acc::default();
        for r in &rows {
            let g = Error::Database;
            let start: DateTime<Utc> = r.try_get("", "start_datetime").map_err(g)?;
            let created: DateTime<Utc> = r.try_get("", "created_at").map_err(g)?;
            let local = start.with_timezone(&tz);
            let p = compute(
                &Inputs {
                    past_missed: r.try_get("", "missed").map_err(g)?,
                    past_attended: r.try_get("", "attended").map_err(g)?,
                    lead_days: ((start - created).num_seconds() as f64 / 86_400.0).max(0.0),
                    age_band: r.try_get("", "age_band").map_err(g)?,
                    weekday: local.weekday().num_days_from_monday() as usize,
                    hour: local.hour(),
                },
                &w,
            )
            .probability;
            let missed = r.try_get::<String>("", "status").map_err(g)? == "no_show";
            for acc in [
                cells
                    .entry((
                        r.try_get("", "q").map_err(g)?,
                        r.try_get("", "lang").map_err(g)?,
                    ))
                    .or_default(),
                &mut all,
            ] {
                acc.n += 1;
                acc.predicted += p;
                acc.high += i64::from(p > high);
                acc.missed += i64::from(missed);
            }
        }
        let keys: Vec<(String, String)> = cells.keys().cloned().collect();
        let counts: Vec<(Vec<(String, String)>, i64)> = keys
            .iter()
            .map(|(q, l)| {
                (
                    vec![
                        ("deprivation_quintile".to_string(), q.clone()),
                        ("language".to_string(), l.clone()),
                    ],
                    cells[&(q.clone(), l.clone())].n,
                )
            })
            .collect();
        let shown = suppress(counts, k);
        let r3 = |x: f64| (x * 1000.0).round() / 1000.0;
        let out: Vec<serde_json::Value> = shown
            .into_iter()
            .zip(keys)
            .map(|(c, key)| {
                let a = &cells[&key];
                if c.suppressed {
                    serde_json::json!({ "deprivation_quintile": key.0, "language": key.1, "suppressed": true })
                } else {
                    let observed = a.missed as f64 / a.n as f64;
                    let predicted = a.predicted / a.n as f64;
                    serde_json::json!({
                        "deprivation_quintile": key.0, "language": key.1, "appointments": a.n,
                        "mean_predicted": r3(predicted), "observed_no_show_rate": r3(observed),
                        "calibration_gap": r3(predicted - observed),
                        "share_in_high_band": r3(a.high as f64 / a.n as f64), "suppressed": false,
                    })
                }
            })
            .collect();
        Ok(serde_json::json!({
            "model_version": MODEL_VERSION, "days": days, "k": k, "high_threshold": high,
            "overall": if all.n == 0 { serde_json::Value::Null } else { serde_json::json!({
                "appointments": all.n, "mean_predicted": r3(all.predicted / all.n as f64),
                "observed_no_show_rate": r3(all.missed as f64 / all.n as f64),
                "share_in_high_band": r3(all.high as f64 / all.n as f64) }) },
            "cells": out,
            "how_to_read": "calibration_gap is mean predicted minus observed no-show rate. A group with a much larger share in the high band than its observed rate supports, or a large gap of either sign, is a reason to reject the model or change its weights. Support only: this model never removes or reorders a slot.",
        }))
    }

    /// A plain description of the model for a reviewer, with the live weights.
    pub fn model_card() -> serde_json::Value {
        let (medium, high) = thresholds();
        serde_json::json!({
            "model_version": MODEL_VERSION,
            "kind": "transparent weighted sum (not trained, not a black box)",
            "purpose": "to tell staff which upcoming appointments may need a supportive phone call; it never changes a booking",
            "factors": ["past completed visits missed (shrunk towards the base rate)", "lead time", "age band", "day of week and part of day (zero by default)"],
            "weights": serde_json::to_value(Weights::from_env_for_card()).unwrap_or_default(),
            "thresholds": { "medium": medium, "high": high },
            "never": ["remove, reorder away or withhold a slot", "overbook", "delay a contact", "act without a person deciding"],
            "gate": "off unless PAS_NOSHOW_SCORE=true and a named person has approved a fairness review and a clinical safety review of this version",
            "not_built": ["distance from the clinic", "clinic type", "weather or travel forecast"],
            "limits": "built from rules, not trained on this population: its numbers are estimates to prompt a conversation, not predictions to rely on",
        })
    }
}

impl Weights {
    fn from_env_for_card() -> serde_json::Value {
        let w = Weights::from_env();
        serde_json::json!({
            "base": w.base, "history": w.history, "prior_visits": w.prior_visits, "lead": w.lead,
            "age": w.age, "weekday": w.weekday, "period": w.period,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Inputs {
        Inputs {
            past_missed: 0,
            past_attended: 0,
            lead_days: 14.0,
            age_band: None,
            weekday: 2,
            hour: 10,
        }
    }

    fn p(i: &Inputs) -> f64 {
        compute(i, &Weights::default()).probability
    }

    #[test]
    fn test_more_past_misses_raise_the_probability() {
        let a = p(&Inputs {
            past_missed: 0,
            past_attended: 6,
            ..base()
        });
        let b = p(&Inputs {
            past_missed: 3,
            past_attended: 3,
            ..base()
        });
        let c = p(&Inputs {
            past_missed: 6,
            past_attended: 0,
            ..base()
        });
        assert!(a < b && b < c, "{a} {b} {c}");
    }

    #[test]
    fn test_a_small_sample_is_not_read_as_strong_evidence() {
        // One miss in two visits is far weaker evidence than five in ten.
        let one_of_two = p(&Inputs {
            past_missed: 1,
            past_attended: 1,
            ..base()
        });
        let five_of_ten = p(&Inputs {
            past_missed: 5,
            past_attended: 5,
            ..base()
        });
        assert!(one_of_two < five_of_ten, "{one_of_two} {five_of_ten}");
        assert!(one_of_two < 0.35);
    }

    #[test]
    fn test_a_longer_lead_time_raises_it_and_saturates_at_ninety_days() {
        let short = p(&Inputs {
            lead_days: 2.0,
            ..base()
        });
        let long = p(&Inputs {
            lead_days: 60.0,
            ..base()
        });
        let capped = p(&Inputs {
            lead_days: 400.0,
            ..base()
        });
        assert!(short < long && long < capped);
        assert_eq!(
            p(&Inputs {
                lead_days: 90.0,
                ..base()
            }),
            capped
        );
    }

    #[test]
    fn test_age_bands_only_raise_it_never_lower_it() {
        let none = p(&base());
        for band in ["16-24", "25-34", "35-44", "75+"] {
            assert!(
                p(&Inputs {
                    age_band: Some(band.into()),
                    ..base()
                }) >= none,
                "{band}"
            );
        }
        assert!(
            p(&Inputs {
                age_band: Some("16-24".into()),
                ..base()
            }) > none
        );
        // The default weights contain no negative adjustment at all.
        let w = Weights::default();
        assert!(
            w.age.values().all(|v| *v >= 0.0)
                && w.weekday.iter().all(|v| *v >= 0.0)
                && w.period.iter().all(|v| *v >= 0.0)
        );
    }

    #[test]
    fn test_day_and_time_matter_only_if_configured() {
        assert_eq!(
            p(&Inputs {
                weekday: 0,
                hour: 8,
                ..base()
            }),
            p(&Inputs {
                weekday: 4,
                hour: 18,
                ..base()
            })
        );
        let mut w = Weights::default();
        w.weekday[0] = 0.05;
        w.period[2] = 0.04;
        let monday_evening = compute(
            &Inputs {
                weekday: 0,
                hour: 18,
                ..base()
            },
            &w,
        )
        .probability;
        let wednesday_morning = compute(
            &Inputs {
                weekday: 2,
                hour: 9,
                ..base()
            },
            &w,
        )
        .probability;
        assert!(monday_evening > wednesday_morning);
    }

    #[test]
    fn test_the_probability_is_always_inside_zero_and_one() {
        let w = Weights {
            history: 5.0,
            lead: 5.0,
            ..Weights::default()
        };
        let hi = compute(
            &Inputs {
                past_missed: 50,
                past_attended: 0,
                lead_days: 90.0,
                age_band: Some("16-24".into()),
                ..base()
            },
            &w,
        )
        .probability;
        assert!((0.01..=0.95).contains(&hi), "{hi}");
        let lo = compute(
            &Inputs {
                past_missed: 0,
                past_attended: 50,
                lead_days: 0.0,
                ..base()
            },
            &Weights {
                base: 0.0,
                ..Weights::default()
            },
        )
        .probability;
        assert!((0.01..=0.95).contains(&lo), "{lo}");
    }

    #[test]
    fn test_the_factors_are_listed_with_their_effect_and_the_sample_size() {
        let a = compute(
            &Inputs {
                past_missed: 2,
                past_attended: 3,
                ..base()
            },
            &Weights::default(),
        );
        assert_eq!(a.sample_size, 5);
        let names: Vec<&str> = a.factors.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "base_rate",
                "past_visits",
                "lead_time",
                "age_band",
                "day_and_time"
            ]
        );
        let past = a.factors.iter().find(|f| f.name == "past_visits").unwrap();
        assert!(past.detail.contains("missed 2 of 5") && past.effect_points > 0.0);
    }

    #[test]
    fn test_the_band_boundaries_are_above_not_at_the_threshold() {
        assert_eq!(
            band(0.70, 0.40, 0.70),
            Band::Medium,
            "exactly 70% is not high"
        );
        assert_eq!(band(0.701, 0.40, 0.70), Band::High);
        assert_eq!(
            band(0.40, 0.40, 0.70),
            Band::Low,
            "exactly 40% is not medium"
        );
        assert_eq!(band(0.41, 0.40, 0.70), Band::Medium);
        // Moving a threshold moves the band without recomputing anything.
        assert_eq!(band(0.55, 0.40, 0.50), Band::High);
    }
}
