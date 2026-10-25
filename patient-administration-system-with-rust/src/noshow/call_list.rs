//! Part of [`super`]: call list.

use super::*;

/// One booked appointment on the call list: the risk and why, and the task a person works from.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct CallItem {
    pub appointment_id: Uuid,
    pub task_id: Uuid,
    /// Present (not null) only once the weights are calibrated and the percentage is switched on.
    #[schema(required = true)]
    pub probability_percent: Option<f64>,
    pub starts_at: DateTime<Utc>,
    pub sample_size: i32,
    pub factors: Vec<Factor>,
}

impl NoShowService {
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
    pub async fn priority_list(&self, limit: i64) -> Result<Vec<CallItem>> {
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
                [crate::paging::fetch(limit).into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                let p: f64 = r.col("probability")?;
                Ok(CallItem {
                    appointment_id: r.col::<Uuid>("appointment_id")?,
                    task_id: r.col::<Uuid>("task_id")?,
                    probability_percent: percent_shown().then(|| (p * 1000.0).round() / 10.0),
                    starts_at: r
                        .try_get::<DateTime<Utc>>("", "start_datetime")
                        .map_err(Error::Database)?,
                    sample_size: r.col::<i32>("sample_size")?,
                    factors: serde_json::from_value(r.col::<serde_json::Value>("factors")?)
                        .map_err(|e| {
                            Error::internal(format!("stored factors are unreadable: {e}"))
                        })?,
                })
            })
            .collect()
    }
}
