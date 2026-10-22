//! Part of [`super`]: read.

use super::*;

pub(super) fn row_to_item(r: &sea_orm::QueryResult) -> Result<Item> {
    Ok(Item {
        id: r.col("id")?,
        campaign_id: r.col("campaign_id")?,
        pathway_id: r.col("pathway_id")?,
        person_id: r.col("person_id")?,
        target_service: r.col("target_service")?,
        state: r.col("state")?,
        answer: r.col("answer")?,
        asked_at: r.col("asked_at")?,
        letter_sent_at: r.col("letter_sent_at")?,
        final_by: r.col("final_by")?,
        decided_by: r.col("decided_by")?,
        removed_at: r.col("removed_at")?,
        next: r.col("next")?,
    })
}

impl ListValidationService {
    pub async fn campaign(&self, id: Uuid) -> Result<Campaign> {
        let r = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, name, service_type, min_weeks, started_by, reply_by, final_wait_days, status, created_at, \
                        COALESCE((SELECT jsonb_object_agg(state, n) FROM \
                           (SELECT state, count(*) AS n FROM validation_items WHERE campaign_id = $1 GROUP BY state) s), '{}'::jsonb) AS counts \
                 FROM validation_campaigns WHERE id = $1",
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("validation campaign {id}")))?;
        Ok(Campaign {
            id: r.col("id")?,
            name: r.col("name")?,
            service_type: r.col("service_type")?,
            min_weeks: r.col("min_weeks")?,
            started_by: r.col("started_by")?,
            reply_by: r.col("reply_by")?,
            final_wait_days: r.col("final_wait_days")?,
            status: r.col("status")?,
            created_at: r.col("created_at")?,
            counts: r.col("counts")?,
        })
    }

    pub async fn campaigns(&self, limit: i64) -> Result<Vec<Uuid>> {
        self.db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id FROM validation_campaigns ORDER BY created_at DESC LIMIT $1",
                [crate::paging::fetch(limit).into()],
            ))
            .await
            .map_err(Error::Database)?
            .iter()
            .map(|r| r.col::<Uuid>("id"))
            .collect()
    }

    /// A campaign's items, optionally one state, or `next=letter_due` / `next=decision_due` for what a person
    /// can act on now. Oldest first.
    pub async fn items(
        &self,
        campaign: Uuid,
        state: Option<&str>,
        next: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Item>> {
        self.campaign(campaign).await?;
        self.db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!(
                    "SELECT {ITEM_COLS} FROM validation_items i JOIN validation_campaigns c ON c.id = i.campaign_id \
                     WHERE i.campaign_id = $1 AND ($2::text IS NULL OR i.state = $2) \
                       AND ($3::text IS NULL OR (CASE WHEN i.state IN ('selected','asked') AND current_date > c.reply_by THEN 'letter_due' \
                            WHEN i.state = 'answered' AND i.answer = 'no_longer_needed' THEN 'decision_due' \
                            WHEN i.state = 'letter_sent' AND current_date > i.final_by THEN 'decision_due' END) = $3) \
                     ORDER BY i.id LIMIT $4"
                ),
                [
                    campaign.into(),
                    state.map(String::from).into(),
                    next.map(String::from).into(),
                    crate::paging::fetch(limit).into(),
                ],
            ))
            .await
            .map_err(Error::Database)?
            .iter()
            .map(row_to_item)
            .collect()
    }

    pub(super) async fn item(&self, campaign: Uuid, item: Uuid) -> Result<Item> {
        self.db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!(
                    "SELECT {ITEM_COLS} FROM validation_items i JOIN validation_campaigns c ON c.id = i.campaign_id \
                     WHERE i.id = $1 AND i.campaign_id = $2"
                ),
                [item.into(), campaign.into()],
            ))
            .await
            .map_err(Error::Database)?
            .map(|r| row_to_item(&r))
            .transpose()?
            .ok_or_else(|| Error::not_found(format!("item {item} in campaign {campaign}")))
    }

    /// Validation removals, apart from treatment: per service, how many were removed, reinstated and kept in
    /// the period (by the date of the decision). No identifiers.
    pub async fn report(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<serde_json::Value>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT target_service, \
                        count(*) FILTER (WHERE state = 'removed')::bigint AS removed, \
                        count(*) FILTER (WHERE state = 'reinstated')::bigint AS reinstated, \
                        count(*) FILTER (WHERE state = 'kept')::bigint AS kept \
                 FROM validation_items WHERE decided_at >= $1 AND decided_at < $2 \
                 GROUP BY target_service ORDER BY target_service",
                [from.into(), to.into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(serde_json::json!({
                    "service": r.col::<String>("target_service")?,
                    "removed": r.col::<i64>("removed")?,
                    "reinstated": r.col::<i64>("reinstated")?,
                    "kept": r.col::<i64>("kept")?,
                }))
            })
            .collect()
    }
}
