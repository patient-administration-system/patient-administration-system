//! Part of [`super`]: budget.

use super::*;
use crate::db::row::RowExt;
use crate::db::sql::{Conn, Stmt, Transact};

impl AnalyticsService {
    /// Per-caller query budget for a day (`PAS_ANALYTICS_QUERY_BUDGET`, default 100).
    ///
    /// Suppression protects one answer. Someone who may ask many differently shaped questions can still
    /// subtract one answer from another to recover a hidden cell (a window that differs by a week, a split that
    /// adds a dimension). The PAS cannot make that impossible for a free-form feed, so it makes it **bounded and
    /// visible**: every query is recorded in the audit log with its shape, and a caller who has used the
    /// day's budget is refused until tomorrow. The rest of the defence is people: who is given the
    /// feed, and the data-sharing terms they sign (`governance/analytics-differencing.md`).
    pub fn query_budget() -> Result<i64> {
        Ok(env::int("PAS_ANALYTICS_QUERY_BUDGET")?)
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
        // Count and charge under a lock held for this caller alone, in one transaction, so two queries at the last
        // unit of the budget cannot both see room for themselves, on one replica or several (audit PA-06).
        let txn = self.db.begin_txn().await?;
        txn.one(Stmt::new(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            [format!("pas.analytics_budget.{who}").into()],
        ))
        .await?;
        let used = txn
            .one(Stmt::new(
                "SELECT count(*)::bigint AS n FROM audit_log WHERE action = 'analytics_query' \
                 AND COALESCE(user_id, 'anonymous') = $1 AND at > now() - interval '24 hours'",
                [who.clone().into()],
            ))
            .await?
            .ok_or_else(|| Error::internal("a count returned no row"))?
            .col::<i64>("n")?;
        if used >= Self::query_budget()? {
            return Err(Error::forbidden(
                "the day's analytics query budget has been used; it renews 24 hours after each query",
            ));
        }
        AuditLogRepository::log(
            &txn,
            "analytics",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, report.as_bytes()),
            "analytics_query",
            None,
            Some(shape),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// The smallest cell the feed will ever show: `PAS_ANALYTICS_K`, default 10, never below 5.
    pub fn minimum_k() -> Result<i64> {
        Ok(env::int("PAS_ANALYTICS_K")?)
    }
}
