//! Part of [`super`]: budget.

use super::*;

impl AnalyticsService {
    /// Per-caller query budget for a day (`PAS_ANALYTICS_QUERY_BUDGET`, default 100).
    ///
    /// Suppression protects one answer. Someone who may ask many differently shaped questions can still
    /// subtract one answer from another to recover a hidden cell (a window that differs by a week, a split that
    /// adds a dimension). The PAS cannot make that impossible for a free-form feed, so it makes it **bounded and
    /// visible**: every query is recorded in the audit log with its shape, and a caller who has used the
    /// day's budget is refused until tomorrow. The rest of the defence is people: who is given the
    /// feed, and the data-sharing terms they sign (`governance/analytics-differencing.md`).
    pub fn query_budget() -> i64 {
        env::int("PAS_ANALYTICS_QUERY_BUDGET")
    }

    /// Record one query against the caller's daily budget, or refuse it. Anonymous callers share one budget.
    pub async fn charge_query(
        &self,
        report: &str,
        shape: serde_json::Value,
        ctx: &UserContext,
    ) -> Result<()> {
        let who = ctx
            .user_id
            .clone()
            .unwrap_or_else(|| "anonymous".to_string());
        let used = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS n FROM audit_log WHERE action = 'analytics_query' \
                 AND COALESCE(user_id, 'anonymous') = $1 AND at > now() - interval '24 hours'",
                [who.clone().into()],
            ))
            .await
            .map_err(Error::Database)?
            .and_then(|r| r.try_get::<i64>("", "n").ok())
            .unwrap_or(0);
        if used >= Self::query_budget() {
            return Err(Error::forbidden(
                "the day's analytics query budget has been used; it renews 24 hours after each query",
            ));
        }
        AuditLogRepository::log(
            &self.db,
            "analytics",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, report.as_bytes()),
            "analytics_query",
            None,
            Some(shape),
            ctx,
        )
        .await
    }

    /// The smallest cell the feed will ever show: `PAS_ANALYTICS_K`, default 10, never below 5.
    pub fn minimum_k() -> i64 {
        env::int("PAS_ANALYTICS_K")
    }
}
