//! Part of [`super`]: status.

use super::*;

impl ToleranceService {
    /// Read every measure over the last `days` days (waits are as of now), for the whole service and then for
    /// each service and ward, and put each in its tier.
    pub async fn status(&self, days: i64) -> Result<Status> {
        let days = days.clamp(1, 365);
        let to = Utc::now();
        let from = to - Duration::days(days);
        let report = self
            .reports
            .clinic_activity(Some(from), Some(to), GroupBy::Day)
            .await?;
        let attended: i64 = report.rows.iter().map(|r| r.attended).sum();
        let missed: i64 = report.rows.iter().map(|r| r.did_not_attend).sum();
        let (beds, occupied) = self.bed_counts().await?;
        let (invoiced, collected, invoices) = self.money(from, to).await?;

        let mut measures = Vec::new();
        let mut whole = |key: &str, value: Option<f64>, sample: i64| {
            if let Some(m) = metric(key) {
                measures.push(Measure {
                    m,
                    scope: Scope::all(),
                    value,
                    sample,
                });
            }
        };
        whole(
            "wait_p90_weeks",
            (report.open_pathway_weeks.n > 0).then_some(report.open_pathway_weeks.p90),
            report.open_pathway_weeks.n,
        );
        whole(
            "lead_time_median_days",
            (report.lead_time_days.n > 0).then_some(report.lead_time_days.median),
            report.lead_time_days.n,
        );
        whole(
            "did_not_attend_rate",
            share(missed, attended + missed),
            attended + missed,
        );
        whole(
            "slot_utilisation",
            (report.total_slots > 0).then_some(report.utilization),
            report.total_slots,
        );
        whole("bed_occupancy", share(occupied, beds), beds);
        whole(
            "invoice_collection_rate",
            (invoiced > 0.0).then(|| (collected / invoiced).min(1.0)),
            invoices,
        );
        measures.extend(self.service_measures(from, to, &report).await?);
        measures.extend(self.ward_measures().await?);

        let rows = self.rows().await?;
        // A scope someone gave its own tolerance is always shown, even with nothing to measure yet.
        for r in rows.iter().filter(|r| r.scope.kind != ScopeKind::All) {
            let Some(m) = metric(&r.metric) else { continue };
            if !measures
                .iter()
                .any(|x| x.m.key == m.key && x.scope == r.scope)
            {
                measures.push(Measure {
                    m,
                    scope: r.scope.clone(),
                    value: None,
                    sample: 0,
                });
            }
        }
        let mut readings = Vec::new();
        let mut by_service = Vec::new();
        let mut by_ward = Vec::new();
        for x in measures {
            let (limits, customised) = Self::resolve(&rows, x.m, &x.scope);
            let reading = Reading {
                metric: x.m.key,
                kind: x.m.kind,
                label: x.m.label,
                unit: x.m.unit,
                direction: x.m.direction,
                scope_kind: x.scope.kind,
                scope_key: x.scope.key.clone(),
                limits,
                value: x.value.map(rounded),
                adverse_drift: x
                    .value
                    .map(|v| rounded(adverse_drift(v, limits.target, x.m.direction))),
                rag: x
                    .value
                    .map_or(Rag::NoData, |v| classify(v, limits, x.m.direction)),
                sample: x.sample,
                customised,
            };
            match x.scope.kind {
                ScopeKind::All => readings.push(reading),
                ScopeKind::Service => by_service.push(reading),
                ScopeKind::Ward => by_ward.push(reading),
            }
        }
        by_service.sort_by(|a, b| (&a.scope_key, a.metric).cmp(&(&b.scope_key, b.metric)));
        by_ward.sort_by(|a, b| (&a.scope_key, a.metric).cmp(&(&b.scope_key, b.metric)));
        let overall = readings
            .iter()
            .chain(&by_service)
            .chain(&by_ward)
            .map(|r| r.rag)
            .max_by_key(|r| r.rank())
            .unwrap_or(Rag::NoData);
        Ok(Status {
            from,
            to,
            overall,
            readings,
            by_service,
            by_ward,
        })
    }

    /// The four service-level measures for each service that has appointments, slots or open pathways.
    pub(super) async fn service_measures(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        report: &crate::reports::Report,
    ) -> Result<Vec<Measure>> {
        let q = |sql: &str| {
            Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql.to_string(),
                [from.into(), to.into()],
            )
        };
        let visits = self
            .db
            .query_all_raw(q(
                "SELECT sch.service_type AS svc, \
                   count(*) FILTER (WHERE a.status IN ('arrived','fulfilled'))::bigint AS attended, \
                   count(*) FILTER (WHERE a.status = 'no_show')::bigint AS missed, \
                   array_agg(floor(greatest(0, extract(epoch FROM (a.start_datetime - a.created_at)) / 86400))::float8) AS leads \
                 FROM appointments a JOIN slots sl ON sl.id = a.slot_id JOIN schedules sch ON sch.id = sl.schedule_id \
                 WHERE a.deleted_at IS NULL AND sch.service_type IS NOT NULL \
                   AND a.start_datetime >= $1 AND a.start_datetime < $2 GROUP BY 1",
            ))
            .await
            .map_err(Error::Database)?;
        let slots = self
            .db
            .query_all_raw(q(
                "SELECT sch.service_type AS svc, count(*)::bigint AS slots, \
                   count(*) FILTER (WHERE sl.status = 'busy')::bigint AS busy \
                 FROM slots sl JOIN schedules sch ON sch.id = sl.schedule_id \
                 WHERE sch.service_type IS NOT NULL AND sl.start_datetime >= $1 AND sl.start_datetime < $2 GROUP BY 1",
            ))
            .await
            .map_err(Error::Database)?;

        use std::collections::BTreeMap;
        #[derive(Default)]
        struct Acc {
            attended: i64,
            missed: i64,
            leads: Vec<f64>,
            slots: i64,
            busy: i64,
            waits: Vec<f64>,
        }
        let mut by: BTreeMap<String, Acc> = BTreeMap::new();
        for r in &visits {
            let acc = by.entry(r.col("svc")?).or_default();
            acc.attended = r.col("attended")?;
            acc.missed = r.col("missed")?;
            acc.leads = r.col("leads")?;
        }
        for r in &slots {
            let acc = by.entry(r.col("svc")?).or_default();
            acc.slots = r.col("slots")?;
            acc.busy = r.col("busy")?;
        }
        for (svc, weeks) in &report.pathway_weeks {
            by.entry(svc.clone()).or_default().waits.push(*weeks);
        }

        let mut out = Vec::new();
        for (svc, acc) in by {
            let scope = Scope {
                kind: ScopeKind::Service,
                key: svc,
            };
            let mut add = |key: &str, value: Option<f64>, sample: i64| {
                if let (Some(m), true) = (metric(key), sample > 0) {
                    out.push(Measure {
                        m,
                        scope: scope.clone(),
                        value,
                        sample,
                    });
                }
            };
            let waits = crate::reports::stats(acc.waits);
            add("wait_p90_weeks", Some(waits.p90), waits.n);
            let leads = crate::reports::stats(acc.leads);
            add("lead_time_median_days", Some(leads.median), leads.n);
            add(
                "did_not_attend_rate",
                share(acc.missed, acc.attended + acc.missed),
                acc.attended + acc.missed,
            );
            add("slot_utilisation", share(acc.busy, acc.slots), acc.slots);
        }
        Ok(out)
    }

    pub(super) async fn ward_measures(&self) -> Result<Vec<Measure>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT w.code AS ward, count(*)::bigint AS beds, \
                   count(*) FILTER (WHERE b.status = 'occupied')::bigint AS occupied \
                 FROM beds b JOIN rooms r ON r.id = b.room_id JOIN wards w ON w.id = r.ward_id GROUP BY w.code"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let Some(m) = metric("bed_occupancy") else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for r in &rows {
            let beds: i64 = r.col("beds")?;
            let occupied: i64 = r.col("occupied")?;
            out.push(Measure {
                m,
                scope: Scope {
                    kind: ScopeKind::Ward,
                    key: r.col("ward")?,
                },
                value: share(occupied, beds),
                sample: beds,
            });
        }
        Ok(out)
    }

    pub(super) async fn bed_counts(&self) -> Result<(i64, i64)> {
        let row = self
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS beds, count(*) FILTER (WHERE status = 'occupied')::bigint AS occupied FROM beds"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let Some(r) = row else { return Ok((0, 0)) };
        Ok((r.col("beds")?, r.col("occupied")?))
    }

    /// Money invoiced and collected on invoices finalised in the window. Amounts are summed as written, so
    /// this assumes one currency; a second currency in the window would be added into the same total.
    pub(super) async fn money(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<(f64, f64, i64)> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT COALESCE(sum(i.total_value::numeric), 0)::float8 AS invoiced, \
                        COALESCE((SELECT sum(p.amount_value::numeric) FROM payments p \
                                  JOIN invoices i2 ON i2.id = p.invoice_id \
                                  WHERE i2.status <> 'void' AND i2.finalized_at >= $1 AND i2.finalized_at < $2), 0)::float8 AS collected, \
                        count(*)::bigint AS n \
                 FROM invoices i WHERE i.status <> 'void' AND i.finalized_at >= $1 AND i.finalized_at < $2",
                [from.into(), to.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let Some(r) = row else {
            return Ok((0.0, 0.0, 0));
        };
        Ok((r.col("invoiced")?, r.col("collected")?, r.col("n")?))
    }
}
