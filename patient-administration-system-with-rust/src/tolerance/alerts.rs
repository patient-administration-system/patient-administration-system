//! Part of [`super`]: alerts.

use super::*;
use crate::db::row::RowExt;
use crate::db::sql::{Conn, Stmt, Transact};

impl ToleranceService {
    /// Check every reading and keep the alerts true to it: a measure that is red has one open alert (and the
    /// `ToleranceAlertRaised` event is published once, when it opens); an alert closes when its measure is
    /// amber or green. A measure with no data leaves its alert as it is, because nothing says it recovered.
    pub async fn evaluate(&self, days: i64) -> Result<Evaluation> {
        let status = self.status(days).await?;
        let readings: Vec<&Reading> = status
            .readings
            .iter()
            .chain(&status.by_service)
            .chain(&status.by_ward)
            .collect();
        let (mut raised, mut cleared) = (0, 0);
        for r in &readings {
            match (r.rag, r.value) {
                (Rag::Red, Some(value)) => {
                    if self.open_or_raise(r, value).await? {
                        raised += 1;
                    }
                }
                (Rag::Green | Rag::Amber, _) => {
                    let n = self
                        .db
                        .run(Stmt::new("UPDATE tolerance_alerts SET cleared_at = now() \
                             WHERE metric = $1 AND scope_kind = $2 AND scope_key = $3 AND cleared_at IS NULL",
                            [
                                r.metric.into(),
                                r.scope_kind.as_str().into(),
                                r.scope_key.clone().into(),
                            ],
                        ))
.await?
                        .rows_affected();
                    cleared += n as usize;
                }
                _ => {}
            }
        }
        let open = self
            .db
            .one(Stmt::text(
                "SELECT count(*)::bigint AS n FROM tolerance_alerts WHERE cleared_at IS NULL"
                    .to_string(),
            ))
            .await?
            .and_then(|r| r.col::<i64>("n").ok())
            .unwrap_or(0);
        Ok(Evaluation {
            checked: readings.len(),
            raised,
            cleared,
            open,
        })
    }

    /// Open an alert for a red reading, or note that it is still red. True when a new alert opened.
    pub(super) async fn open_or_raise(&self, r: &Reading, value: f64) -> Result<bool> {
        let txn = self.db.begin_txn().await?;
        let id = Uuid::new_v4();
        let inserted = txn
            .run(Stmt::new("INSERT INTO tolerance_alerts (id, metric, scope_kind, scope_key, value, target, tolerance, escalate_beyond) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING",
                [
                    id.into(),
                    r.metric.into(),
                    r.scope_kind.as_str().into(),
                    r.scope_key.clone().into(),
                    value.into(),
                    r.limits.target.into(),
                    r.limits.tolerance.into(),
                    r.limits.escalate_beyond.into(),
                ],
            ))
.await?
            .rows_affected()
            == 1;
        if inserted {
            OutboxRepository::publish(
                &txn,
                "ToleranceAlertRaised",
                &serde_json::json!({
                    "alert_id": id,
                    "metric": r.metric,
                    "scope_kind": r.scope_kind,
                    "scope_key": r.scope_key,
                    "value": value,
                    "target": r.limits.target,
                    "escalate_beyond": r.limits.escalate_beyond,
                }),
            )
            .await?;
        } else {
            txn.run(Stmt::new(
                "UPDATE tolerance_alerts SET last_seen_at = now(), value = $4 \
                 WHERE metric = $1 AND scope_kind = $2 AND scope_key = $3 AND cleared_at IS NULL",
                [
                    r.metric.into(),
                    r.scope_kind.as_str().into(),
                    r.scope_key.clone().into(),
                    value.into(),
                ],
            ))
            .await?;
        }
        txn.commit().await.map_err(Error::Database)?;
        Ok(inserted)
    }

    /// Alerts, newest first: the open ones, or every one, up to `limit` (see [`crate::paging`]).
    pub async fn alerts(&self, open_only: bool, limit: i64) -> Result<Vec<Alert>> {
        let rows = self
            .db
            .all(Stmt::new(format!(
                    "SELECT id, metric, scope_kind, scope_key, value, target, tolerance, escalate_beyond, raised_at, \
                            last_seen_at, cleared_at, acknowledged_by, acknowledged_at, note \
                     FROM tolerance_alerts {} ORDER BY raised_at DESC, id DESC LIMIT $1",
                    if open_only { "WHERE cleared_at IS NULL" } else { "" }
                ),
                [crate::paging::fetch(limit).into()],
            ))
.await?;
        let mut out = Vec::new();
        for r in &rows {
            let key: String = r.col("metric")?;
            let m = metric(&key);
            out.push(Alert {
                id: r.col("id")?,
                label: m.map(|m| m.label),
                unit: m.map(|m| m.unit),
                metric: key,
                scope_kind: r.col("scope_kind")?,
                scope_key: r.col("scope_key")?,
                value: r.col("value")?,
                target: r.col("target")?,
                tolerance: r.col("tolerance")?,
                escalate_beyond: r.col("escalate_beyond")?,
                raised_at: r.col("raised_at")?,
                last_seen_at: r.col("last_seen_at")?,
                cleared_at: r.col("cleared_at")?,
                acknowledged_by: r.col("acknowledged_by")?,
                acknowledged_at: r.col("acknowledged_at")?,
                note: r.col("note")?,
            });
        }
        Ok(out)
    }

    /// A named person says they are looking at an open alert. It stays open until the measure is no longer red.
    /// Once; audited.
    pub async fn acknowledge(
        &self,
        id: Uuid,
        by: &str,
        note: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = crate::validation::named_person(
            by,
            "name the person acknowledging the alert (acknowledged_by)",
        )?;
        let n = self
            .db
            .run(Stmt::new("UPDATE tolerance_alerts SET acknowledged_by = $2, acknowledged_at = now(), note = $3 \
                 WHERE id = $1 AND cleared_at IS NULL AND acknowledged_by IS NULL",
                [id.into(), by.into(), note.trim().into()],
            ))
.await?
            .rows_affected();
        if n == 0 {
            let exists = self
                .db
                .one(Stmt::new(
                    "SELECT 1 AS x FROM tolerance_alerts WHERE id = $1",
                    [id.into()],
                ))
                .await?
                .is_some();
            return Err(if exists {
                Error::conflict("the alert is closed or already acknowledged")
            } else {
                Error::not_found("no such alert")
            });
        }
        AuditLogRepository::log(
            &self.db,
            "tolerance_alert",
            id,
            "acknowledge",
            None,
            Some(serde_json::json!({ "acknowledged_by": by })),
            ctx,
        )
        .await
    }
}
