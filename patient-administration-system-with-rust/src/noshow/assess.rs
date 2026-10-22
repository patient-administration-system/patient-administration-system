//! Part of [`super`]: assess.

use super::*;

impl NoShowService {
    pub(super) async fn inputs_for(
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
        let start: DateTime<Utc> = r.col("start_datetime")?;
        let created: DateTime<Utc> = r.col("created_at")?;
        let updated: DateTime<Utc> = r.col("updated_at")?;
        let local = start.with_timezone(&tz);
        Ok(Some((
            r.col("person_id")?,
            Inputs {
                past_missed: r.col("missed")?,
                past_attended: r.col("attended")?,
                lead_days: ((start - created).num_seconds() as f64 / 86_400.0).max(0.0),
                age_band: r.col("age_band")?,
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

    pub(super) async fn stored(&self, appointment_id: Uuid) -> Result<Option<Shown>> {
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
                let p: f64 = r.col("probability")?;
                let factors: serde_json::Value = r.col("factors")?;
                Ok(Shown {
                    appointment_id,
                    probability: p,
                    probability_percent: percent_shown().then(|| (p * 1000.0).round() / 10.0),
                    band: if r.col::<bool>("listed")? {
                        Band::High
                    } else {
                        band(p, medium)
                    },
                    factors: serde_json::from_value(factors).unwrap_or_default(),
                    sample_size: i64::from(r.col::<i32>("sample_size")?),
                    model_version: r.col("model_version")?,
                    computed_at: r.col("computed_at")?,
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
    pub(super) async fn visible(&self, ids: &[Uuid]) -> Result<std::collections::HashSet<Uuid>> {
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
        rows.iter().map(|r| r.col::<Uuid>("id")).collect()
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
            let id: Uuid = r.col("id")?;
            self.assess(id).await?;
            n += 1;
        }
        self.refresh_call_list().await?;
        Ok(n)
    }
}
