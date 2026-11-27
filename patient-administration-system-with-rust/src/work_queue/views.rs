//! Saved views of the work queue (TSC-7): a named filter (queue, owner, priority, overdue, kind) saved by a
//! person, which they may share with a role. A view holds the filter, never the items, so it returns exactly
//! what `GET /api/work-items` returns for the same filter; `owner = me` is the person looking at it.
//!
//! A view is seen by the person who saved it and, once shared, by anyone whose verified token carries that
//! role. Only the person who saved it may share, unshare, or delete it, and each of those is audited.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Filter, HIGH, LOW, WorkItem, WorkKind, WorkQueueService, check_queue};
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::row::RowExt;
use crate::db::sql::{Conn, FromRow, Row, Stmt, Transact};
use crate::{Error, Result};

/// One saved view, as the API sends it.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct WorkView {
    pub id: Uuid,
    pub name: String,
    /// Who saved it; only they may share or delete it.
    pub created_by: String,
    #[schema(required = true)]
    pub queue: Option<String>,
    /// `me` (whoever looks), `none` (nobody has the item), or a person's name.
    #[schema(required = true)]
    pub owner: Option<String>,
    #[schema(required = true)]
    pub priority: Option<i16>,
    pub overdue: bool,
    #[schema(required = true, value_type = Option<String>)]
    pub kind: Option<WorkKind>,
    /// The role it is shared with, or none (only its maker sees it).
    #[schema(required = true)]
    pub shared_with_role: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl FromRow for WorkView {
    fn from_row(r: &Row) -> Result<Self> {
        Ok(WorkView {
            id: r.col("id")?,
            name: r.col("name")?,
            created_by: r.col("created_by")?,
            queue: r.col("queue")?,
            owner: r.col("owner_filter")?,
            priority: r.col("priority")?,
            overdue: r.col("overdue")?,
            kind: r
                .col::<Option<String>>("kind")?
                .map(|k| k.parse())
                .transpose()?,
            shared_with_role: r.col("shared_with_role")?,
            created_at: r.col("created_at")?,
            updated_at: r.col("updated_at")?,
        })
    }
}

/// A view to save.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct NewView {
    pub name: String,
    pub queue: Option<String>,
    /// `me`, `none`, or a person's name.
    pub owner: Option<String>,
    pub priority: Option<i16>,
    #[serde(default)]
    pub overdue: bool,
    #[schema(value_type = Option<String>)]
    pub kind: Option<WorkKind>,
}

impl WorkView {
    /// The filter this view stands for, as `viewer` sees it.
    pub fn filter_for(&self, viewer: &str) -> Filter {
        let mut f = Filter {
            queue: self.queue.clone(),
            overdue: self.overdue,
            priority: self.priority,
            kind: self.kind,
            ..Filter::default()
        };
        match self.owner.as_deref() {
            None => {}
            Some("none") => f.unclaimed = true,
            Some("me") => f.owner = Some(viewer.to_string()),
            Some(name) => f.owner = Some(name.to_string()),
        }
        f
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "name": self.name, "shared_with_role": self.shared_with_role })
    }
}

fn check_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 100 {
        return Err(Error::validation("a view's name is 1 to 100 characters"));
    }
    Ok(n.to_string())
}

impl WorkQueueService {
    /// Save a view for `who`. A name is once per person.
    pub async fn save_view(&self, who: &str, v: NewView, ctx: &UserContext) -> Result<WorkView> {
        let who = crate::validation::named_person(who, "saving a view needs a signed-in person")?;
        let name = check_name(&v.name)?;
        if let Some(q) = &v.queue {
            check_queue(q)?;
        }
        if let Some(p) = v.priority
            && !(HIGH..=LOW).contains(&p)
        {
            return Err(Error::validation("priority is 1 (high) to 3 (low)"));
        }
        let owner = v
            .owner
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty());
        let txn = self.db.begin_txn().await?;
        let row = txn
            .one(Stmt::new(
                "INSERT INTO work_views (id, name, created_by, queue, owner_filter, priority, overdue, kind) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (created_by, name) DO NOTHING RETURNING *",
                [
                    Uuid::new_v4().into(),
                    name.into(),
                    who.into(),
                    v.queue.into(),
                    owner.into(),
                    v.priority.into(),
                    v.overdue.into(),
                    v.kind.map(|k| k.as_str().to_string()).into(),
                ],
            ))
            .await?
            .ok_or_else(|| Error::conflict("you already have a view with that name"))?;
        let view = WorkView::from_row(&row)?;
        AuditLogRepository::log(
            &txn,
            "work_view",
            view.id,
            "create",
            None,
            Some(view.snapshot()),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(view)
    }

    /// The views `who` may see: their own and those shared with one of `roles`, by name.
    pub async fn views_for(&self, who: &str, roles: &[String]) -> Result<Vec<WorkView>> {
        self.db
            .all_as::<WorkView>(Stmt::new(
                "SELECT * FROM work_views WHERE created_by = $1 OR shared_with_role = ANY($2) \
                 ORDER BY name, created_by, id LIMIT 500",
                [who.into(), roles.to_vec().into()],
            ))
            .await
    }

    /// One view, if `who` may see it; otherwise not found (a view one may not see is not said to exist).
    pub async fn view(&self, id: Uuid, who: &str, roles: &[String]) -> Result<WorkView> {
        self.db
            .one_as::<WorkView>(Stmt::new(
                "SELECT * FROM work_views WHERE id = $1 AND (created_by = $2 OR shared_with_role = ANY($3))",
                [id.into(), who.into(), roles.to_vec().into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("work view {id}")))
    }

    /// The items of a view, as `who` sees them: the same as listing with its filter.
    pub async fn view_items(
        &self,
        id: Uuid,
        who: &str,
        roles: &[String],
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<WorkItem>> {
        let view = self.view(id, who, roles).await?;
        self.list(&view.filter_for(who), rows).await
    }

    /// Share a view with a role, or stop sharing it (`None`). Only the person who saved it; audited.
    pub async fn share_view(
        &self,
        id: Uuid,
        who: &str,
        role: Option<&str>,
        ctx: &UserContext,
    ) -> Result<WorkView> {
        let who = crate::validation::named_person(who, "sharing a view needs a signed-in person")?;
        let role = role.map(str::trim).filter(|r| !r.is_empty());
        if role.is_some_and(|r| r.chars().count() > 100) {
            return Err(Error::validation("a role is at most 100 characters"));
        }
        let before = self.own_view(id, who).await?;
        let txn = self.db.begin_txn().await?;
        let row = txn
            .one(Stmt::new(
                "UPDATE work_views SET shared_with_role = $2, updated_at = now() WHERE id = $1 RETURNING *",
                [id.into(), role.map(str::to_string).into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("work view {id}")))?;
        let after = WorkView::from_row(&row)?;
        AuditLogRepository::log(
            &txn,
            "work_view",
            id,
            if role.is_some() { "share" } else { "unshare" },
            Some(before.snapshot()),
            Some(after.snapshot()),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(after)
    }

    /// Delete a view. Only the person who saved it; audited.
    pub async fn delete_view(&self, id: Uuid, who: &str, ctx: &UserContext) -> Result<()> {
        let who = crate::validation::named_person(who, "deleting a view needs a signed-in person")?;
        let before = self.own_view(id, who).await?;
        let txn = self.db.begin_txn().await?;
        txn.run(Stmt::new(
            "DELETE FROM work_views WHERE id = $1",
            [id.into()],
        ))
        .await?;
        AuditLogRepository::log(
            &txn,
            "work_view",
            id,
            "delete",
            Some(before.snapshot()),
            None,
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(())
    }

    /// A view `who` saved; one they may only see is a `409`, one they may not see is not found.
    async fn own_view(&self, id: Uuid, who: &str) -> Result<WorkView> {
        let view = self
            .db
            .one_as::<WorkView>(Stmt::new(
                "SELECT * FROM work_views WHERE id = $1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("work view {id}")))?;
        if view.created_by != who {
            return Err(Error::conflict(
                "only the person who saved a view may change it",
            ));
        }
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(owner: Option<&str>) -> WorkView {
        WorkView {
            id: Uuid::nil(),
            name: "v".into(),
            created_by: "Ann".into(),
            queue: Some("booking".into()),
            owner: owner.map(str::to_string),
            priority: Some(1),
            overdue: true,
            kind: Some(WorkKind::Call),
            shared_with_role: None,
            created_at: Timestamp::UNIX_EPOCH,
            updated_at: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn a_view_is_its_filter_and_me_is_whoever_looks() {
        let f = view(Some("me")).filter_for("Bob");
        assert_eq!(f.owner.as_deref(), Some("Bob"));
        assert_eq!(
            (f.queue.as_deref(), f.priority, f.overdue, f.kind),
            (Some("booking"), Some(1), true, Some(WorkKind::Call))
        );
        assert!(view(Some("none")).filter_for("Bob").unclaimed);
        assert_eq!(
            view(Some("Cat")).filter_for("Bob").owner.as_deref(),
            Some("Cat")
        );
        assert_eq!(view(None).filter_for("Bob").owner, None);
    }

    #[test]
    fn a_name_is_one_to_a_hundred_characters() {
        assert!(check_name("  ").is_err());
        assert!(check_name(&"x".repeat(101)).is_err());
        assert_eq!(check_name(" Overdue calls ").unwrap(), "Overdue calls");
    }
}
