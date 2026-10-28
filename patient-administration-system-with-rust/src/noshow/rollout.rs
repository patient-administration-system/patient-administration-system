//! Part of [`super`]: rollout.

use super::*;

/// One clinic's stage: recording in shadow, or live, and whether it has been in shadow long enough to go live.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct RolloutRow {
    pub service_type: String,
    /// `shadow` or `live`.
    pub stage: String,
    #[schema(required = true)]
    pub changed_by: Option<String>,
    #[schema(required = true)]
    pub live_since: Option<DateTime<Utc>>,
    pub assessments: i64,
    pub days_in_shadow: i64,
    pub may_go_live: bool,
}

impl NoShowService {
    /// Every clinic seen so far with its stage and how long it has been recording in shadow.
    pub async fn rollout(&self) -> Result<Vec<RolloutRow>> {
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
                let first: DateTime<Utc> = r.col("first_at")?;
                let days = (Utc::now() - first).num_days();
                Ok(RolloutRow {
                    service_type: r.col::<String>("svc")?,
                    stage: r.col::<String>("stage")?,
                    changed_by: r
                        .try_get::<Option<String>>("", "changed_by")
                        .map_err(Error::Database)?,
                    live_since: r
                        .try_get::<Option<DateTime<Utc>>>("", "live_since")
                        .map_err(Error::Database)?,
                    assessments: r.col::<i64>("n")?,
                    days_in_shadow: days,
                    may_go_live: days >= min,
                })
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
    pub async fn evaluation(&self, days: i64, k: Option<i64>) -> Result<Evaluation> {
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
                    r.col::<i64>("n")?,
                ))
            })
            .collect::<Result<_>>()?;
        let cells = suppress(counts, k);
        let calibration: Vec<CalibrationCell> = rows
            .iter()
            .zip(cells)
            .map(|(r, c)| {
                let t = c.dims[0].1.clone();
                if c.suppressed {
                    CalibrationCell {
                        predicted_tenth: t,
                        appointments: None,
                        mean_predicted: None,
                        observed_no_show_rate: None,
                        gap: None,
                        suppressed: true,
                    }
                } else {
                    let (p, o): (f64, f64) = (
                        r.try_get("", "predicted").unwrap_or(0.0),
                        r.try_get("", "observed").unwrap_or(0.0),
                    );
                    CalibrationCell {
                        predicted_tenth: t,
                        appointments: c.count,
                        mean_predicted: Some(r3(p)),
                        observed_no_show_rate: Some(r3(o)),
                        gap: Some(r3(p - o)),
                        suppressed: false,
                    }
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
        let window = |m: i64, n: i64| RateWindow {
            // A rate is shown only if every count behind it is at least k, so a hidden figure cannot be
            // recovered from a published one.
            no_show_rate: (n >= k && n > 0).then(|| r3(m as f64 / n as f64)),
            appointments: (n >= k).then_some(n),
        };
        let clinics: Vec<ClinicEffect> = clinic_rows
            .iter()
            .map(|r| {
                let g = |c: &str| r.try_get::<i64>("", c).unwrap_or(0);
                ClinicEffect {
                    service_type: r.try_get::<String>("", "svc").unwrap_or_default(),
                    live_since: r.try_get::<DateTime<Utc>>("", "live_since").ok(),
                    before: window(g("before_missed"), g("before_n")),
                    after: window(g("after_missed"), g("after_n")),
                    same_period_clinics_still_in_shadow: window(g("shadow_missed"), g("shadow_n")),
                }
            })
            .collect();
        Ok(Evaluation {
            model_version: MODEL_VERSION.into(),
            days: i64::from(days),
            k,
            calibration,
            clinics,
            how_to_read: "Calibration compares the probability recorded before each visit with what happened; read it before showing any percentage. Clinics compares the no-show rate before and after a clinic went live with the clinics still in shadow over the same period; a fall in the live clinics that is not matched in the others is the evidence the score helps. Nobody is withheld from support to make a comparison: the standard reminders apply everywhere and the score only adds calls.".into(),
        })
    }
}

/// Predicted against observed for one tenth of predicted probability. A tenth under the minimum size is
/// listed with `suppressed: true` and none of the figures.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CalibrationCell {
    pub predicted_tenth: String,
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
    pub gap: Option<f64>,
    pub suppressed: bool,
}

/// A no-show rate over a period, or `null` for either figure when a count behind it is under the minimum.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RateWindow {
    #[schema(required = true)]
    pub no_show_rate: Option<f64>,
    #[schema(required = true)]
    pub appointments: Option<i64>,
}

/// One clinic's rate before and after it went live, beside the clinics still in shadow.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ClinicEffect {
    pub service_type: String,
    #[schema(required = true)]
    pub live_since: Option<DateTime<Utc>>,
    pub before: RateWindow,
    pub after: RateWindow,
    pub same_period_clinics_still_in_shadow: RateWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Evaluation {
    pub model_version: String,
    pub days: i64,
    pub k: i64,
    pub calibration: Vec<CalibrationCell>,
    pub clinics: Vec<ClinicEffect>,
    pub how_to_read: String,
}
