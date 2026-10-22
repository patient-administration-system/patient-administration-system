//! Part of [`super`]: settings.

use super::*;

impl ToleranceService {
    pub(super) async fn rows(&self) -> Result<Vec<Row>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT metric, scope_kind, scope_key, target, tolerance, escalate_beyond, set_by, note, updated_at \
                 FROM tolerances ORDER BY metric, scope_kind, scope_key"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in &rows {
            let kind: String = r.col("scope_kind")?;
            let Some(kind) = ScopeKind::parse(&kind) else {
                continue;
            };
            out.push(Row {
                scope: Scope {
                    kind,
                    key: r.col("scope_key")?,
                },
                metric: r.col("metric")?,
                limits: Limits {
                    target: r.col("target")?,
                    tolerance: r.col("tolerance")?,
                    escalate_beyond: r.col("escalate_beyond")?,
                },
                set_by: r.try_get("", "set_by").ok(),
                note: r.try_get("", "note").ok(),
                updated_at: r.try_get("", "updated_at").ok(),
            });
        }
        Ok(out)
    }

    /// The limits that apply to a measure in a scope: the scope's own, else the whole service's, else the
    /// built-in default. The flag says whether a person set the one used.
    pub(super) fn resolve(rows: &[Row], m: &Metric, scope: &Scope) -> (Limits, bool) {
        let find = |s: &Scope| rows.iter().find(|r| r.metric == m.key && &r.scope == s);
        find(scope)
            .or_else(|| find(&Scope::all()))
            .map_or((m.default, false), |r| (r.limits, true))
    }

    /// Every measure with the settings that apply to the whole service, then each setting a person made for
    /// one service or ward.
    pub async fn settings(&self) -> Result<Vec<Setting>> {
        let rows = self.rows().await?;
        let make = |m: &Metric, row: Option<&Row>, scope: Scope| Setting {
            metric: m.key,
            kind: m.kind,
            label: m.label,
            unit: m.unit,
            direction: m.direction,
            scope_kind: scope.kind,
            scope_key: scope.key,
            limits: row.map_or(m.default, |r| r.limits),
            customised: row.is_some(),
            set_by: row.and_then(|r| r.set_by.clone()),
            note: row.and_then(|r| r.note.clone()),
            updated_at: row.and_then(|r| r.updated_at),
        };
        let mut out = Vec::new();
        for m in METRICS {
            let whole = rows
                .iter()
                .find(|r| r.metric == m.key && r.scope.kind == ScopeKind::All);
            out.push(make(&m, whole, Scope::all()));
        }
        for r in rows.iter().filter(|r| r.scope.kind != ScopeKind::All) {
            if let Some(m) = metric(&r.metric) {
                out.push(make(m, Some(r), r.scope.clone()));
            }
        }
        Ok(out)
    }

    /// A named person sets a measure's target, tolerance and limit, for the whole service or for one service
    /// or ward. Audited.
    pub async fn set(
        &self,
        key: &str,
        scope: &Scope,
        limits: Limits,
        set_by: &str,
        note: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        let m = metric(key).ok_or_else(|| Error::not_found(format!("no such measure: {key}")))?;
        check_scope(m, scope)?;
        let by = crate::validation::named_person(
            set_by,
            "name the person setting the tolerance (set_by)",
        )?;
        check(limits)?;
        let before = self
            .rows()
            .await?
            .into_iter()
            .find(|r| r.metric == m.key && &r.scope == scope)
            .map(|r| r.limits);
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO tolerances (metric, scope_kind, scope_key, target, tolerance, escalate_beyond, set_by, note) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
                 ON CONFLICT (metric, scope_kind, scope_key) DO UPDATE SET target = $4, tolerance = $5, \
                   escalate_beyond = $6, set_by = $7, note = $8, updated_at = now()",
                [
                    m.key.into(),
                    scope.kind.as_str().into(),
                    scope.key.clone().into(),
                    limits.target.into(),
                    limits.tolerance.into(),
                    limits.escalate_beyond.into(),
                    by.into(),
                    note.trim().into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "tolerance",
            scope.uuid(m.key),
            "set",
            before.and_then(|l| serde_json::to_value(l).ok()),
            serde_json::to_value(limits).ok(),
            ctx,
        )
        .await
    }

    /// Take away what a person set: the scope goes back to the whole service's limits, and the whole service
    /// back to the built-in default. Audited.
    pub async fn reset(&self, key: &str, scope: &Scope, ctx: &UserContext) -> Result<()> {
        let m = metric(key).ok_or_else(|| Error::not_found(format!("no such measure: {key}")))?;
        check_scope(m, scope)?;
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "DELETE FROM tolerances WHERE metric = $1 AND scope_kind = $2 AND scope_key = $3",
                [
                    m.key.into(),
                    scope.kind.as_str().into(),
                    scope.key.clone().into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log(
            &self.db,
            "tolerance",
            scope.uuid(m.key),
            "reset",
            None,
            serde_json::to_value(m.default).ok(),
            ctx,
        )
        .await
    }
}
