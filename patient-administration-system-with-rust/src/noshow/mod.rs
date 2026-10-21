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
use crate::config::env;
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

/// Low up to the medium threshold, medium above it (`> 0.40` is medium, `0.40` is not). **High is not a
/// probability cut-off:** it means "on today's call list", which is the top few matched to the people
/// available to phone (see [`NoShowService::refresh_call_list`]). A fixed high cut-off with a low base rate
/// can only be reached by patients who have missed before, which would make the list a "missed before" list.
pub fn band(p: f64, medium: f64) -> Band {
    if p > medium { Band::Medium } else { Band::Low }
}

/// How many people the clinic can phone in a day (`PAS_NOSHOW_CALL_CAPACITY`, default 10, at most 500).
pub fn call_capacity() -> i64 {
    env::int("PAS_NOSHOW_CALL_CAPACITY")
}

/// Days of shadow data a clinic needs before it may go live (`PAS_NOSHOW_SHADOW_MIN_DAYS`, default 28).
pub fn shadow_min_days() -> i64 {
    env::int("PAS_NOSHOW_SHADOW_MIN_DAYS")
}

/// How far ahead the call list looks (`PAS_NOSHOW_CALL_HORIZON_DAYS`, default 7).
pub fn call_horizon_days() -> i64 {
    env::int("PAS_NOSHOW_CALL_HORIZON_DAYS")
}

/// The percentage is hidden until someone sets this after calibrating the weights on real outcomes
/// (`PAS_NOSHOW_SHOW_PERCENT=true`). Until then staff see the band word and the reasons only.
pub fn percent_shown() -> bool {
    env::flag("PAS_NOSHOW_SHOW_PERCENT")
}

pub fn medium_threshold() -> f64 {
    env::float("PAS_NOSHOW_MEDIUM")
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
    /// Added by age band. Empty by default. Bands may only raise the probability, so support goes where it is needed.
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
            // Age is not an input by default. A deployment may add bands, but then the fairness reviewer
            // must say why in the approval (see `record_review`).
            age: BTreeMap::new(),
            weekday: [0.0; 7],
            period: [0.0; 3],
        }
    }
}

impl Weights {
    /// `PAS_NOSHOW_WEIGHTS` as JSON overriding any of the defaults; an unreadable value keeps the defaults.
    pub fn from_env() -> Self {
        env::text("PAS_NOSHOW_WEIGHTS")
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
    pub call_capacity: i64,
    pub call_horizon_days: i64,
    /// Whether the percentage is shown to staff (off until calibrated).
    pub percentage_shown: bool,
    /// Whether any age weight is in use (then the fairness approval must justify it).
    pub age_in_use: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Shown {
    pub appointment_id: Uuid,
    /// For the server's own use (tests, the audit); never serialised.
    #[serde(skip)]
    pub probability: f64,
    /// Present only once the weights are calibrated and `PAS_NOSHOW_SHOW_PERCENT=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probability_percent: Option<f64>,
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
        env::flag("PAS_NOSHOW_SCORE")
    }

    async fn latest_review(&self, kind: &str, valid_days: i64) -> Result<Option<String>> {
        Ok(self
            .db
            .query_one_raw(Statement::from_sql_and_values(
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
        Ok(Status {
            model_version: MODEL_VERSION,
            switched_on,
            fairness_approved: fairness,
            clinical_safety_approved: safety,
            active,
            reason,
            medium_threshold: medium_threshold(),
            call_capacity: call_capacity(),
            call_horizon_days: call_horizon_days(),
            percentage_shown: percent_shown(),
            age_in_use: Weights::from_env().age.values().any(|v| *v != 0.0),
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
        let notes = notes.trim();
        let reviewer = crate::validation::named_person(
            reviewer,
            "a review is made by a named person (reviewer)",
        )?;
        if outcome == "approved" && notes.chars().count() < 40 {
            return Err(Error::validation(
                "an approval needs notes (at least 40 characters) saying what was looked at and why it is acceptable",
            ));
        }
        if outcome == "approved"
            && kind == "fairness"
            && Weights::from_env().age.values().any(|v| *v != 0.0)
            && !notes
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == "age" || w == "ages" || w == "aged")
        {
            return Err(Error::validation(
                "age bands carry weight in this configuration: a fairness approval must say why using age is justified (the notes must discuss age)",
            ));
        }
        if notes.is_empty() || notes.chars().count() > 1000 {
            return Err(Error::validation("notes must be 1 to 1000 characters"));
        }
        let id = Uuid::new_v4();
        self.db
            .execute_raw(Statement::from_sql_and_values(
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
            .query_one_raw(Statement::from_sql_and_values(
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
                .query_one_raw(Statement::from_sql_and_values(
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
            .execute_raw(Statement::from_sql_and_values(
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
        let _ = person;
        self.stored(appointment_id)
            .await?
            .ok_or_else(|| Error::internal("assessment vanished"))
    }

    async fn stored(&self, appointment_id: Uuid) -> Result<Option<Shown>> {
        let medium = medium_threshold();
        self.db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT probability, factors, sample_size, model_version, computed_at, \
                   EXISTS (SELECT 1 FROM staff_call_tasks t WHERE t.appointment_id = noshow_assessments.appointment_id AND t.reason = 'high_risk') AS listed \
                 FROM noshow_assessments WHERE appointment_id = $1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .map(|r| {
                let p: f64 = r.try_get("", "probability").map_err(Error::Database)?;
                let factors: serde_json::Value = r.try_get("", "factors").map_err(Error::Database)?;
                Ok(Shown {
                    appointment_id,
                    probability: p,
                    probability_percent: percent_shown().then(|| (p * 1000.0).round() / 10.0),
                    band: if r.try_get::<bool>("", "listed").map_err(Error::Database)? {
                        Band::High
                    } else {
                        band(p, medium)
                    },
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
        let ids: Vec<Uuid> = ids.iter().take(500).copied().collect();
        let visible = self.visible(&ids).await?;
        let mut out = Vec::new();
        for id in ids.iter().filter(|i| visible.contains(i)) {
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

    /// The same, for a person at the front desk: refused for a clinic still in shadow mode, where the
    /// probability is recorded to be checked but not shown.
    pub async fn for_staff(&self, appointment_id: Uuid) -> Result<Shown> {
        self.require_active().await?;
        if !self
            .visible(&[appointment_id])
            .await?
            .contains(&appointment_id)
        {
            return Err(Error::conflict(
                "this clinic is in shadow mode: the probability is being recorded and checked, not shown",
            ));
        }
        self.assess(appointment_id).await
    }

    /// Which of these appointments belong to a clinic that is live.
    async fn visible(&self, ids: &[Uuid]) -> Result<std::collections::HashSet<Uuid>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT a.id FROM appointments a JOIN slots sl ON sl.id = a.slot_id \
                 JOIN schedules sc ON sc.id = sl.schedule_id \
                 JOIN noshow_rollout r ON r.service_type = sc.service_type AND r.stage = 'live' \
                 WHERE a.id = ANY($1)",
                [ids.to_vec().into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| r.try_get::<Uuid>("", "id").map_err(Error::Database))
            .collect()
    }

    /// Every clinic seen so far with its stage and how long it has been recording in shadow.
    pub async fn rollout(&self) -> Result<Vec<serde_json::Value>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT s.svc, COALESCE(r.stage, 'shadow') AS stage, r.changed_by, r.live_since, s.first_at, s.n \
                 FROM (SELECT sc.service_type AS svc, min(n.computed_at) AS first_at, count(*) AS n \
                       FROM noshow_assessments n JOIN appointments a ON a.id = n.appointment_id \
                       JOIN slots sl ON sl.id = a.slot_id JOIN schedules sc ON sc.id = sl.schedule_id GROUP BY 1) s \
                 LEFT JOIN noshow_rollout r ON r.service_type = s.svc ORDER BY s.svc"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let min = shadow_min_days();
        rows.iter()
            .map(|r| {
                let first: DateTime<Utc> = r.try_get("", "first_at").map_err(Error::Database)?;
                let days = (Utc::now() - first).num_days();
                Ok(serde_json::json!({
                    "service_type": r.try_get::<String>("", "svc").map_err(Error::Database)?,
                    "stage": r.try_get::<String>("", "stage").map_err(Error::Database)?,
                    "changed_by": r.try_get::<Option<String>>("", "changed_by").map_err(Error::Database)?,
                    "live_since": r.try_get::<Option<DateTime<Utc>>>("", "live_since").map_err(Error::Database)?,
                    "assessments": r.try_get::<i64>("", "n").map_err(Error::Database)?,
                    "days_in_shadow": days,
                    "may_go_live": days >= min,
                }))
            })
            .collect()
    }

    /// Move one clinic between shadow and live. Going live needs: the score active (both reviews), a named
    /// person, a note saying why this clinic now, and at least [`shadow_min_days`] of shadow data for it,
    /// so the first real use of the score at a clinic follows a period of checking it against what happened.
    /// Going back to shadow is always allowed; calls already on the list stay for a person to clear.
    pub async fn set_stage(
        &self,
        service_type: &str,
        stage: &str,
        changed_by: &str,
        note: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        if !["shadow", "live"].contains(&stage) {
            return Err(Error::validation("stage must be shadow or live"));
        }
        let (svc, by, note) = (service_type.trim(), changed_by.trim(), note.trim());
        if svc.is_empty() || by.is_empty() {
            return Err(Error::validation(
                "name the clinic (service type) and the person making the change",
            ));
        }
        if stage == "live" {
            if note.chars().count() < 20 {
                return Err(Error::validation(
                    "going live needs a note (20 characters or more) saying why this clinic, now",
                ));
            }
            self.require_active().await?;
            let row = self
                .db
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT min(n.computed_at) AS first_at FROM noshow_assessments n \
                     JOIN appointments a ON a.id = n.appointment_id JOIN slots sl ON sl.id = a.slot_id \
                     JOIN schedules sc ON sc.id = sl.schedule_id WHERE sc.service_type = $1",
                    [svc.into()],
                ))
                .await
                .map_err(Error::Database)?;
            let first: Option<DateTime<Utc>> = row.and_then(|r| r.try_get("", "first_at").ok());
            let days = first.map_or(0, |f| (Utc::now() - f).num_days());
            if days < shadow_min_days() {
                return Err(Error::conflict(format!(
                    "{svc} has {days} days of shadow data; at least {} are needed before it goes live",
                    shadow_min_days()
                )));
            }
        }
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO noshow_rollout (service_type, stage, changed_by, note, live_since) \
                 VALUES ($1,$2,$3,$4, CASE WHEN $2 = 'live' THEN now() END) \
                 ON CONFLICT (service_type) DO UPDATE SET stage = $2, changed_by = $3, note = $4, changed_at = now(), \
                   live_since = CASE WHEN $2 = 'live' THEN COALESCE(noshow_rollout.live_since, now()) ELSE noshow_rollout.live_since END",
                [svc.into(), stage.into(), by.into(), note.into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log_with(
            &self.db,
            "noshow_model",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, svc.as_bytes()),
            &format!("rollout_{stage}"),
            None,
            Some(serde_json::json!({ "service_type": svc, "changed_by": by })),
            Some(stage),
            None,
            ctx,
        )
        .await
    }

    /// Is it working? Two views, both with small cells withheld and no identifiers:
    ///
    /// - **calibration**: the stored probability (made before the visit, which shadow mode guarantees) against
    ///   what happened, by tenth of predicted. This is what a person reads before setting
    ///   `PAS_NOSHOW_SHOW_PERCENT`, and what shows whether the weights are any good.
    /// - **clinics**: for each clinic, the no-show rate before and after it went live, next to the rate at the
    ///   clinics still in shadow over the same period. The stepped rollout makes the not-yet-live clinics the
    ///   comparison, so nobody is held back from support to make one. A difference that shows in the live
    ///   clinics and not in the others is the evidence the score raises attendance; a drift in both is not.
    pub async fn evaluation(&self, days: i64, k: Option<i64>) -> Result<serde_json::Value> {
        let k = k.unwrap_or(0).max(AnalyticsService::minimum_k());
        let days = days.clamp(7, 1825) as i32;
        let r3 = |x: f64| (x * 1000.0).round() / 1000.0;
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT floor(n.probability * 10)::int AS tenth, count(*)::bigint AS n, avg(n.probability) AS predicted, \
                   avg((a.status = 'no_show')::int) AS observed \
                 FROM noshow_assessments n JOIN appointments a ON a.id = n.appointment_id \
                 WHERE a.status IN ('fulfilled','arrived','no_show') AND a.start_datetime > now() - make_interval(days => $1) \
                 GROUP BY 1 ORDER BY 1",
                [days.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let counts: Vec<(Vec<(String, String)>, i64)> = rows
            .iter()
            .map(|r| {
                let t: i32 = r.try_get("", "tenth").unwrap_or(0);
                Ok((
                    vec![("tenth".to_string(), t.min(9).to_string())],
                    r.try_get::<i64>("", "n").map_err(Error::Database)?,
                ))
            })
            .collect::<Result<_>>()?;
        let cells = suppress(counts, k);
        let calibration: Vec<serde_json::Value> = rows
            .iter()
            .zip(cells)
            .map(|(r, c)| {
                let t = &c.dims[0].1;
                if c.suppressed {
                    serde_json::json!({ "predicted_tenth": t, "suppressed": true })
                } else {
                    let (p, o): (f64, f64) = (r.try_get("", "predicted").unwrap_or(0.0), r.try_get("", "observed").unwrap_or(0.0));
                    serde_json::json!({ "predicted_tenth": t, "appointments": c.count, "mean_predicted": r3(p),
                        "observed_no_show_rate": r3(o), "gap": r3(p - o), "suppressed": false })
                }
            })
            .collect();

        // Per clinic: before and after the go-live moment, and the same after-period at clinics still in shadow.
        let clinic_rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "WITH o AS ( \
                   SELECT COALESCE(sc.service_type,'unknown') AS svc, a.start_datetime AS at, (a.status = 'no_show') AS missed \
                   FROM appointments a LEFT JOIN slots sl ON sl.id = a.slot_id LEFT JOIN schedules sc ON sc.id = sl.schedule_id \
                   WHERE a.deleted_at IS NULL AND a.status IN ('fulfilled','arrived','no_show') \
                     AND a.start_datetime > now() - make_interval(days => $1)) \
                 SELECT r.service_type AS svc, r.live_since, \
                   count(*) FILTER (WHERE o.at < r.live_since)::bigint AS before_n, \
                   count(*) FILTER (WHERE o.at < r.live_since AND o.missed)::bigint AS before_missed, \
                   count(*) FILTER (WHERE o.at >= r.live_since)::bigint AS after_n, \
                   count(*) FILTER (WHERE o.at >= r.live_since AND o.missed)::bigint AS after_missed, \
                   (SELECT count(*) FROM o o2 WHERE o2.at >= r.live_since AND NOT EXISTS \
                      (SELECT 1 FROM noshow_rollout x WHERE x.service_type = o2.svc AND x.stage = 'live'))::bigint AS shadow_n, \
                   (SELECT count(*) FROM o o2 WHERE o2.at >= r.live_since AND o2.missed AND NOT EXISTS \
                      (SELECT 1 FROM noshow_rollout x WHERE x.service_type = o2.svc AND x.stage = 'live'))::bigint AS shadow_missed \
                 FROM noshow_rollout r LEFT JOIN o ON o.svc = r.service_type \
                 WHERE r.live_since IS NOT NULL GROUP BY r.service_type, r.live_since ORDER BY r.service_type",
                [days.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let rate = |m: i64, n: i64, ok: bool| {
            if ok && n > 0 {
                serde_json::json!(r3(m as f64 / n as f64))
            } else {
                serde_json::Value::Null
            }
        };
        let clinics: Vec<serde_json::Value> = clinic_rows
            .iter()
            .map(|r| {
                let g = |c: &str| r.try_get::<i64>("", c).unwrap_or(0);
                // A rate is shown only if every count behind it is at least k, so a hidden figure cannot be
                // recovered from a published one.
                let (bn, an, sn) = (g("before_n"), g("after_n"), g("shadow_n"));
                serde_json::json!({
                    "service_type": r.try_get::<String>("", "svc").unwrap_or_default(),
                    "live_since": r.try_get::<DateTime<Utc>>("", "live_since").ok(),
                    "before": { "no_show_rate": rate(g("before_missed"), bn, bn >= k), "appointments": (bn >= k).then_some(bn) },
                    "after": { "no_show_rate": rate(g("after_missed"), an, an >= k), "appointments": (an >= k).then_some(an) },
                    "same_period_clinics_still_in_shadow": { "no_show_rate": rate(g("shadow_missed"), sn, sn >= k), "appointments": (sn >= k).then_some(sn) },
                })
            })
            .collect();
        Ok(serde_json::json!({
            "model_version": MODEL_VERSION, "days": days, "k": k,
            "calibration": calibration, "clinics": clinics,
            "how_to_read": "Calibration compares the probability recorded before each visit with what happened; read it before showing any percentage. Clinics compares the no-show rate before and after a clinic went live with the clinics still in shadow over the same period; a fall in the live clinics that is not matched in the others is the evidence the score helps. Nobody is withheld from support to make a comparison: the standard reminders apply everywhere and the score only adds calls.",
        }))
    }

    /// The daily job: assess every booked appointment still to come. Idempotent within a day.
    pub async fn recalculate_all(&self) -> Result<usize> {
        self.require_active().await?;
        let ids = self
            .db
            .query_all_raw(Statement::from_string(
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
        self.refresh_call_list().await?;
        Ok(n)
    }

    /// Today's call list: the top few by probability among booked appointments in the next
    /// [`call_horizon_days`], as many as the clinic can phone ([`call_capacity`]) less what is still open.
    /// Matching the list to capacity, not to a probability cut-off, is deliberate (see [`band`]). It adds a task
    /// for a person to ring; it never touches a booking. Idempotent: an appointment is listed once.
    pub async fn refresh_call_list(&self) -> Result<u64> {
        self.require_active().await?;
        let r = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) \
                 SELECT gen_random_uuid(), a.id, a.person_id, 'high_risk', GREATEST(now(), a.start_datetime - interval '2 days') \
                 FROM noshow_assessments n JOIN appointments a ON a.id = n.appointment_id \
                 JOIN slots sl ON sl.id = a.slot_id JOIN schedules sc ON sc.id = sl.schedule_id \
                 JOIN noshow_rollout r ON r.service_type = sc.service_type AND r.stage = 'live' \
                 WHERE a.status = 'booked' AND a.deleted_at IS NULL AND a.start_datetime > now() \
                   AND a.start_datetime < now() + make_interval(days => $1::int) \
                   AND NOT EXISTS (SELECT 1 FROM staff_call_tasks t WHERE t.appointment_id = a.id AND t.reason = 'high_risk') \
                 ORDER BY n.probability DESC, a.start_datetime \
                 LIMIT GREATEST(0, $2::bigint - (SELECT count(*) FROM staff_call_tasks WHERE reason = 'high_risk' AND handled_at IS NULL)) \
                 ON CONFLICT (appointment_id, reason) DO NOTHING",
                [(call_horizon_days() as i32).into(), call_capacity().into()],
            ))
            .await
            .map_err(Error::Database)?;
        Ok(r.rows_affected())
    }

    /// The call list: booked appointments a person has not yet dealt with, highest first, with the factors.
    pub async fn priority_list(&self, limit: i64) -> Result<Vec<serde_json::Value>> {
        self.require_active().await?;
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT n.appointment_id, n.probability, n.factors, n.sample_size, a.start_datetime, t.id AS task_id \
                 FROM noshow_assessments n JOIN appointments a ON a.id = n.appointment_id \
                 JOIN staff_call_tasks t ON t.appointment_id = n.appointment_id AND t.reason = 'high_risk' \
                 WHERE a.status = 'booked' AND a.start_datetime > now() AND t.handled_at IS NULL \
                 ORDER BY n.probability DESC, a.start_datetime LIMIT $1",
                [limit.clamp(1, 500).into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                let p: f64 = r.try_get("", "probability").map_err(Error::Database)?;
                Ok(serde_json::json!({
                    "appointment_id": r.try_get::<Uuid>("", "appointment_id").map_err(Error::Database)?,
                    "task_id": r.try_get::<Uuid>("", "task_id").map_err(Error::Database)?,
                    "probability_percent": percent_shown().then(|| (p * 1000.0).round() / 10.0),
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
            .query_all_raw(Statement::from_sql_and_values(
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
        // Pass 1: the model's probability for each past appointment, as it would have been.
        let mut scored: Vec<(f64, bool, (String, String))> = Vec::new();
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
            scored.push((
                p,
                missed,
                (
                    r.try_get("", "q").map_err(g)?,
                    r.try_get("", "lang").map_err(g)?,
                ),
            ));
        }
        // The call list takes the top few, not everyone above a fixed probability, so the audit asks the same
        // question: who would be in the top tenth of the period? (A tie at the cut-off is all in.)
        let mut ps: Vec<f64> = scored.iter().map(|x| x.0).collect();
        ps.sort_by(|a, b| b.total_cmp(a));
        let cutoff = ps
            .get(ps.len().div_ceil(10).saturating_sub(1))
            .copied()
            .unwrap_or(1.0);
        // Pass 2: accumulate by group.
        for (p, missed, key) in scored {
            for acc in [cells.entry(key).or_default(), &mut all] {
                acc.n += 1;
                acc.predicted += p;
                acc.high += i64::from(p >= cutoff);
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
                        "share_in_top_tenth": r3(a.high as f64 / a.n as f64), "suppressed": false,
                    })
                }
            })
            .collect();
        Ok(serde_json::json!({
            "model_version": MODEL_VERSION, "days": days, "k": k, "top_tenth_cutoff": r3(cutoff),
            "overall": if all.n == 0 { serde_json::Value::Null } else { serde_json::json!({
                "appointments": all.n, "mean_predicted": r3(all.predicted / all.n as f64),
                "observed_no_show_rate": r3(all.missed as f64 / all.n as f64),
                "share_in_top_tenth": r3(all.high as f64 / all.n as f64) }) },
            "cells": out,
            "how_to_read": "calibration_gap is mean predicted minus observed no-show rate. A group with a much larger share in the top tenth than its observed rate supports, or a large gap of either sign, is a reason to reject the model or change its weights. Support only: this model never removes or reorders a slot.",
        }))
    }

    /// A plain description of the model for a reviewer, with the live weights.
    pub fn model_card() -> serde_json::Value {
        let medium = medium_threshold();
        serde_json::json!({
            "model_version": MODEL_VERSION,
            "kind": "transparent weighted sum (not trained, not a black box)",
            "purpose": "to tell staff which upcoming appointments may need a supportive phone call; it never changes a booking",
            "factors": ["past completed visits missed (shrunk towards the base rate)", "lead time", "age band (zero by default: not an input unless a fairness reviewer justifies it)", "day of week and part of day (zero by default)"],
            "weights": serde_json::to_value(Weights::from_env_for_card()).unwrap_or_default(),
            "bands": { "medium_above": medium, "high": "on today's call list: the top few by probability, as many as the clinic can phone (PAS_NOSHOW_CALL_CAPACITY, default 10) among appointments in the next PAS_NOSHOW_CALL_HORIZON_DAYS (7); not a probability cut-off" },
            "percentage": "hidden from staff until the weights are calibrated on real outcomes and PAS_NOSHOW_SHOW_PERCENT=true; staff see the band word and the reasons",
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
        // Age is not an input by default: every band scores the same as no band.
        assert!(Weights::default().age.is_empty());
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
    fn test_the_band_boundary_is_above_not_at_the_threshold() {
        assert_eq!(band(0.40, 0.40), Band::Low, "exactly 40% is not medium");
        assert_eq!(band(0.41, 0.40), Band::Medium);
        // Moving the threshold moves the band without recomputing anything.
        assert_eq!(band(0.30, 0.20), Band::Medium);
    }
}
