//! Part of [`super`]: model.

use super::*;

/// Low up to the medium threshold, medium above it (`> 0.40` is medium, `0.40` is not). **High is not a
/// probability cut-off:** it means "on today's call list", which is the top few matched to the people
/// available to phone (see the server's `NoShowService::refresh_call_list`). A fixed high cut-off with a low base rate
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

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
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
    let weekday = w.weekday.get(i.weekday.min(6)).copied().unwrap_or(0.0);
    let period = w
        .period
        .get(match i.hour {
            0..=11 => 0,
            12..=16 => 1,
            _ => 2,
        })
        .copied()
        .unwrap_or(0.0);
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

impl Weights {
    pub fn from_env_for_card() -> WeightsCard {
        let w = Weights::from_env();
        WeightsCard {
            base: w.base,
            history: w.history,
            prior_visits: w.prior_visits,
            lead: w.lead,
            age: w.age,
            weekday: w.weekday,
            period: w.period,
        }
    }
}

/// The live weights, as the model card shows them. The fields are those of [`Weights`].
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WeightsCard {
    pub base: f64,
    pub history: f64,
    pub prior_visits: f64,
    pub lead: f64,
    pub age: BTreeMap<String, f64>,
    pub weekday: [f64; 7],
    pub period: [f64; 3],
}

#[cfg(test)]
mod env_tests {
    use super::*;

    /// Each variable here is private to one test in this module (`PAS_NOSHOW_MEDIUM` is read, never set,
    /// because `config::env`'s tests set it).
    fn with_var<T>(name: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let before = std::env::var(name).ok();
        // SAFETY: test-only, and the variable is private to the calling test.
        unsafe {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        let out = f();
        unsafe {
            match before {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        out
    }

    #[test]
    fn the_settings_have_their_documented_defaults() {
        assert_eq!(
            with_var("PAS_NOSHOW_CALL_CAPACITY", None, call_capacity),
            10
        );
        assert_eq!(
            with_var("PAS_NOSHOW_SHADOW_MIN_DAYS", None, shadow_min_days),
            28
        );
        assert_eq!(
            with_var("PAS_NOSHOW_CALL_HORIZON_DAYS", None, call_horizon_days),
            7
        );
        assert!(!with_var("PAS_NOSHOW_SHOW_PERCENT", None, percent_shown));
        assert!((0.0..=1.0).contains(&medium_threshold()));
    }

    #[test]
    fn the_settings_read_their_variables() {
        assert_eq!(
            with_var("PAS_NOSHOW_CALL_CAPACITY", Some("25"), call_capacity),
            25
        );
        assert_eq!(
            with_var("PAS_NOSHOW_CALL_CAPACITY", Some("501"), call_capacity),
            10,
            "above the maximum of 500"
        );
        assert!(with_var(
            "PAS_NOSHOW_SHOW_PERCENT",
            Some("true"),
            percent_shown
        ));
    }

    #[test]
    fn weights_come_from_json_or_fall_back_to_the_defaults() {
        let w = with_var(
            "PAS_NOSHOW_WEIGHTS",
            Some(r#"{"base":0.2,"weekday":[0.1,0,0,0,0,0,0]}"#),
            Weights::from_env,
        );
        assert!((w.base - 0.2).abs() < 1e-9);
        assert!((w.weekday[0] - 0.1).abs() < 1e-9);
        assert!(
            (w.history - 0.9).abs() < 1e-9,
            "unnamed fields keep defaults"
        );
        let d = Weights::default();
        for bad in [None, Some("not json")] {
            let w = with_var("PAS_NOSHOW_WEIGHTS", bad, Weights::from_env);
            assert!((w.base - d.base).abs() < 1e-9);
        }
        let card = with_var(
            "PAS_NOSHOW_WEIGHTS",
            Some(r#"{"lead":0.5}"#),
            Weights::from_env_for_card,
        );
        assert!((card.lead - 0.5).abs() < 1e-9);
        assert!((card.base - d.base).abs() < 1e-9);
        assert_eq!(card.period, d.period);
    }
}
