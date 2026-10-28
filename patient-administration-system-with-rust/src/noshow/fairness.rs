//! Part of [`super`]: fairness.

use super::*;

impl NoShowService {
    /// Backtest the model on appointments that have already had an outcome, as the model would have scored
    /// them with only what was known then, split by deprivation quintile and language. For a reviewer: it
    /// shows whether the model's predictions fall harder on one group than the outcomes do. Cells under k are
    /// withheld. This runs whether or not the score is switched on: evidence comes before approval.
    pub async fn fairness_audit(&self, days: i64, k: Option<i64>) -> Result<FairnessAudit> {
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
            let start: DateTime<Utc> = r.col("start_datetime")?;
            let created: DateTime<Utc> = r.col("created_at")?;
            let local = start.with_timezone(&tz);
            let p = compute(
                &Inputs {
                    past_missed: r.col("missed")?,
                    past_attended: r.col("attended")?,
                    lead_days: ((start - created).num_seconds() as f64 / 86_400.0).max(0.0),
                    age_band: r.col("age_band")?,
                    weekday: local.weekday().num_days_from_monday() as usize,
                    hour: local.hour(),
                },
                &w,
            )
            .probability;
            let missed = r.col::<String>("status")? == "no_show";
            scored.push((p, missed, (r.col("q")?, r.col("lang")?)));
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
        let out: Vec<FairnessCell> = shown
            .into_iter()
            .zip(keys)
            .map(|(c, key)| {
                let a = &cells[&key];
                if c.suppressed {
                    FairnessCell {
                        deprivation_quintile: key.0,
                        language: key.1,
                        appointments: None,
                        mean_predicted: None,
                        observed_no_show_rate: None,
                        calibration_gap: None,
                        share_in_top_tenth: None,
                        suppressed: true,
                    }
                } else {
                    let observed = a.missed as f64 / a.n as f64;
                    let predicted = a.predicted / a.n as f64;
                    FairnessCell {
                        deprivation_quintile: key.0,
                        language: key.1,
                        appointments: Some(a.n),
                        mean_predicted: Some(r3(predicted)),
                        observed_no_show_rate: Some(r3(observed)),
                        calibration_gap: Some(r3(predicted - observed)),
                        share_in_top_tenth: Some(r3(a.high as f64 / a.n as f64)),
                        suppressed: false,
                    }
                }
            })
            .collect();
        Ok(FairnessAudit {
            model_version: MODEL_VERSION.into(),
            days,
            k,
            top_tenth_cutoff: r3(cutoff),
            overall: (all.n > 0).then(|| FairnessOverall {
                appointments: all.n,
                mean_predicted: r3(all.predicted / all.n as f64),
                observed_no_show_rate: r3(all.missed as f64 / all.n as f64),
                share_in_top_tenth: r3(all.high as f64 / all.n as f64),
            }),
            cells: out,
            how_to_read: "calibration_gap is mean predicted minus observed no-show rate. A group with a much larger share in the top tenth than its observed rate supports, or a large gap of either sign, is a reason to reject the model or change its weights. Support only: this model never removes or reorders a slot.".into(),
        })
    }

    /// A plain description of the model for a reviewer, with the live weights.
    pub fn model_card() -> ModelCard {
        let medium = medium_threshold();
        let words = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        ModelCard {
            model_version: MODEL_VERSION.into(),
            kind: "transparent weighted sum (not trained, not a black box)".into(),
            purpose: "to tell staff which upcoming appointments may need a supportive phone call; it never changes a booking".into(),
            factors: words(&["past completed visits missed (shrunk towards the base rate)", "lead time", "age band (zero by default: not an input unless a fairness reviewer justifies it)", "day of week and part of day (zero by default)"]),
            weights: Weights::from_env_for_card(),
            bands: ModelBands {
                medium_above: medium,
                high: "on today's call list: the top few by probability, as many as the clinic can phone (PAS_NOSHOW_CALL_CAPACITY, default 10) among appointments in the next PAS_NOSHOW_CALL_HORIZON_DAYS (7); not a probability cut-off".into(),
            },
            percentage: "hidden from staff until the weights are calibrated on real outcomes and PAS_NOSHOW_SHOW_PERCENT=true; staff see the band word and the reasons".into(),
            never: words(&["remove, reorder away or withhold a slot", "overbook", "delay a contact", "act without a person deciding"]),
            gate: "off unless PAS_NOSHOW_SCORE=true and a named person has approved a fairness review and a clinical safety review of this version".into(),
            not_built: words(&["distance from the clinic", "clinic type", "weather or travel forecast"]),
            limits: "built from rules, not trained on this population: its numbers are estimates to prompt a conversation, not predictions to rely on".into(),
        }
    }
}

/// One group in the fairness audit. A cell under the minimum size is listed with `suppressed: true` and
/// none of the figures.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FairnessCell {
    pub deprivation_quintile: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub appointments: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub mean_predicted: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub observed_no_show_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub calibration_gap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub share_in_top_tenth: Option<f64>,
    pub suppressed: bool,
}

/// The audit over every appointment in the period.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FairnessOverall {
    pub appointments: i64,
    pub mean_predicted: f64,
    pub observed_no_show_rate: f64,
    pub share_in_top_tenth: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FairnessAudit {
    pub model_version: String,
    pub days: i64,
    pub k: i64,
    pub top_tenth_cutoff: f64,
    /// `null` when there were no appointments with an outcome in the period.
    #[schema(required = true)]
    pub overall: Option<FairnessOverall>,
    pub cells: Vec<FairnessCell>,
    pub how_to_read: String,
}

/// How the risk bands are cut.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ModelBands {
    pub medium_above: f64,
    pub high: String,
}

/// A plain description of the model for a reviewer.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ModelCard {
    pub model_version: String,
    pub kind: String,
    pub purpose: String,
    pub factors: Vec<String>,
    pub weights: WeightsCard,
    pub bands: ModelBands,
    pub percentage: String,
    pub never: Vec<String>,
    pub gate: String,
    pub not_built: Vec<String>,
    pub limits: String,
}
