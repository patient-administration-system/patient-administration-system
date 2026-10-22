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

use crate::db::row::RowExt;
use chrono::{DateTime, Datelike, Timelike, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::analytics::{AnalyticsService, suppress};
use crate::config::env;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

mod assess;
mod call_list;
mod fairness;
mod model;
mod review;
mod rollout;
pub use model::*;

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
