//! Part of [`super`]: reports.

use super::*;

impl AnalyticsService {
    /// Refill rate of released slots, split by the chosen dimensions (PILOT-10). Releases are counted by the
    /// day they were released; only those with a known outcome count (refilled, or the slot's time passed
    /// unfilled), so a slot still open for next week does not look like a failure. A cell under `k`
    /// resolved releases is withheld whole, and the same secondary suppression as [`Self::outcomes`] applies.
    pub async fn refill_rates(
        &self,
        group_by: &[RefillDim],
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        k: Option<i64>,
    ) -> Result<RefillReport> {
        if group_by.is_empty() || group_by.len() > 4 {
            return Err(Error::validation("group_by takes one to four dimensions"));
        }
        let mut seen = Vec::new();
        for d in group_by {
            if seen.contains(d) {
                return Err(Error::validation("a dimension may be named once"));
            }
            seen.push(*d);
        }
        let k = k.unwrap_or(0).max(Self::minimum_k());
        // Release times are stamped by the database's clock, which can be a little ahead of this server's, so
        // the default end is a few minutes ahead of now rather than exactly now.
        let to = to.unwrap_or_else(|| Utc::now() + chrono::Duration::minutes(5));
        let from = from.unwrap_or_else(|| to - chrono::Duration::days(90));
        let select: Vec<String> = group_by
            .iter()
            .enumerate()
            .map(|(i, d)| format!("{} AS d{i}", d.sql()))
            .collect();
        let group: Vec<String> = (1..=group_by.len()).map(|i| i.to_string()).collect();
        let sql = format!(
            "SELECT {sel}, count(*)::bigint AS n, \
                    count(*) FILTER (WHERE r.status = 'refilled')::bigint AS refilled, \
                    count(*) FILTER (WHERE r.release_class = 'early_release')::bigint AS early \
             FROM slot_releases r \
             LEFT JOIN appointments a ON a.id = r.appointment_id \
             LEFT JOIN patient_communication_profiles cp ON cp.person_id = a.person_id \
             LEFT JOIN person_demographics d ON d.person_id = a.person_id \
             WHERE r.released_at >= $1 AND r.released_at < $2 \
               AND (r.status = 'refilled' OR r.start_datetime <= now()) \
             GROUP BY {grp} ORDER BY {grp}",
            sel = select.join(", "),
            grp = group.join(", "),
        );
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                [from.into(), to.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut data = Vec::new();
        let mut extra = Vec::new();
        let (mut total, mut total_refilled) = (0, 0);
        for r in &rows {
            let dims = group_by
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    Ok((
                        d.name().to_string(),
                        r.try_get::<String>("", &format!("d{i}"))
                            .map_err(Error::Database)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let n: i64 = r.col("n")?;
            let refilled: i64 = r.col("refilled")?;
            let early: i64 = r.col("early")?;
            total += n;
            total_refilled += refilled;
            data.push((dims, n));
            extra.push((refilled, early));
        }
        let flags = suppress(data, k);
        let cells: Vec<RefillCell> = flags
            .into_iter()
            .zip(extra)
            .map(|(c, (refilled, early))| match c.count {
                Some(n) => RefillCell {
                    dims: c.dims,
                    resolved: Some(n),
                    refilled: Some(refilled),
                    refill_rate: rate(refilled, n),
                    early_release_share: rate(early, n),
                    suppressed: false,
                },
                None => RefillCell {
                    dims: c.dims,
                    resolved: None,
                    refilled: None,
                    refill_rate: None,
                    early_release_share: None,
                    suppressed: true,
                },
            })
            .collect();
        Ok(RefillReport {
            group_by: group_by.iter().map(|d| d.name().to_string()).collect(),
            k,
            from,
            to,
            total_resolved: total,
            overall_refill_rate: rate(total_refilled, total),
            suppressed_cells: cells.iter().filter(|c| c.suppressed).count(),
            cells,
        })
    }

    pub async fn outcomes(
        &self,
        group_by: &[Dim],
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        k: Option<i64>,
    ) -> Result<Report> {
        if group_by.is_empty() || group_by.len() > 4 {
            return Err(Error::validation("group_by takes one to four dimensions"));
        }
        let mut seen = Vec::new();
        for d in group_by {
            if seen.contains(d) {
                return Err(Error::validation("a dimension may be named once"));
            }
            seen.push(*d);
        }
        let k = k.unwrap_or(0).max(Self::minimum_k());
        let to = to.unwrap_or_else(|| Utc::now() + chrono::Duration::minutes(5));
        let from = from.unwrap_or_else(|| to - chrono::Duration::days(90));
        let tz = crate::inbound::messages::timezone().name().to_string();

        let select: Vec<String> = group_by
            .iter()
            .enumerate()
            .map(|(i, d)| format!("{} AS d{i}", d.sql()))
            .collect();
        let group: Vec<String> = (1..=group_by.len()).map(|i| i.to_string()).collect();
        let sql = format!(
            "SELECT {sel}, count(*)::bigint AS n \
             FROM appointments a \
             LEFT JOIN slots sl ON sl.id = a.slot_id \
             LEFT JOIN schedules sc ON sc.id = sl.schedule_id \
             LEFT JOIN patient_communication_profiles cp ON cp.person_id = a.person_id \
             LEFT JOIN person_demographics d ON d.person_id = a.person_id \
             WHERE a.deleted_at IS NULL AND a.start_datetime >= $1 AND a.start_datetime < $2 \
             GROUP BY {grp} ORDER BY {grp}",
            sel = select.join(", "),
            grp = group.join(", "),
        );
        // The time zone is a parameter only when a dimension uses it.
        let mut values: Vec<sea_orm::Value> = vec![from.into(), to.into()];
        if group_by.contains(&Dim::TimeOfDay) {
            values.push(tz.into());
        }
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                values,
            ))
            .await
            .map_err(Error::Database)?;
        let mut data = Vec::new();
        let mut total = 0;
        for r in &rows {
            let dims = group_by
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    Ok((
                        d.name().to_string(),
                        r.try_get::<String>("", &format!("d{i}"))
                            .map_err(Error::Database)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let n: i64 = r.col("n")?;
            total += n;
            data.push((dims, n));
        }
        let cells = suppress(data, k);
        Ok(Report {
            group_by: group_by.iter().map(|d| d.name().to_string()).collect(),
            k,
            from,
            to,
            total,
            suppressed_cells: cells.iter().filter(|c| c.suppressed).count(),
            cells,
        })
    }
}
