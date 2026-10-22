//! Part of [`super`]: decision.

use super::*;

impl ListValidationService {
    /// A named person decides: `keep` the patient on the list, or `remove` them. Only when the patient said
    /// the care is no longer needed, or did not answer the letter by its date. Never for a patient who said
    /// they still need it or are unsure, and never on an urgent service.
    pub async fn decide(
        &self,
        campaign: Uuid,
        item: Uuid,
        decision: &str,
        decided_by: &str,
        note: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Item> {
        if !["keep", "remove"].contains(&decision) {
            return Err(Error::validation("decision must be keep or remove"));
        }
        let by = crate::validation::named_person(
            decided_by,
            "a decision is made by a named person (decided_by)",
        )?
        .to_string();
        if note.is_some_and(|n| n.chars().count() > 500) {
            return Err(Error::validation("note must be at most 500 characters"));
        }
        let (decision_s, note_s, ctx) =
            (decision.to_string(), note.map(str::to_string), ctx.clone());
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .query_one_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT i.state, i.answer, i.person_id, i.pathway_id, i.target_service, \
                                    CASE WHEN i.state IN ('selected','asked') AND current_date > c.reply_by THEN 'letter_due' \
                                         WHEN i.state = 'answered' AND i.answer = 'no_longer_needed' THEN 'decision_due' \
                                         WHEN i.state = 'letter_sent' AND current_date > i.final_by THEN 'decision_due' END AS next, \
                                    EXISTS (SELECT 1 FROM follow_up_policies p WHERE p.service_type = i.target_service) AS urgent \
                             FROM validation_items i JOIN validation_campaigns c ON c.id = i.campaign_id \
                             WHERE i.id = $1 AND i.campaign_id = $2 FOR UPDATE OF i",
                            [item.into(), campaign.into()],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .ok_or_else(|| Error::not_found(format!("item {item} in campaign {campaign}")))?;
                    let state: String = r.col("state")?;
                    let answer: Option<String> = r.col("answer")?;
                    let next: Option<String> = r.col("next")?;
                    let urgent: bool = r.col("urgent")?;
                    if decision_s == "remove" {
                        if urgent {
                            return Err(Error::conflict(
                                "an urgent pathway is never removed by validation: a clinician decides",
                            ));
                        }
                        if matches!(answer.as_deref(), Some("still_needed" | "unsure")) {
                            return Err(Error::conflict(
                                "the patient said they still need this or are unsure: they are not removed",
                            ));
                        }
                    }
                    if next.as_deref() != Some("decision_due") {
                        // A patient who answered still_needed may be recorded as kept at any time.
                        let closing_a_yes = decision_s == "keep" && state == "answered";
                        if !closing_a_yes {
                            return Err(Error::conflict(format!(
                                "not ready for a decision: the item is {state}{}",
                                if state == "letter_sent" {
                                    " and the wait for a reply to the letter has not ended"
                                } else {
                                    " and has no answer and no letter yet"
                                }
                            )));
                        }
                    }
                    let person: Uuid = r.col("person_id")?;
                    let pathway: Uuid = r.col("pathway_id")?;
                    let service: String = r.col("target_service")?;
                    if decision_s == "keep" {
                        txn.execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE validation_items SET state = 'kept', decided_by = $2, decided_at = now(), decision_note = $3 WHERE id = $1",
                            [item.into(), by.clone().into(), note_s.clone().into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    } else {
                        let stopped_at = Utc::now();
                        let removed = txn
                            .query_all_raw(Statement::from_sql_and_values(
                                DbBackend::Postgres,
                                "UPDATE waitlist_entries SET status = 'removed', updated_at = now() \
                                 WHERE person_id = $1 AND target_service = $2 AND status = 'waiting' RETURNING id",
                                [person.into(), service.clone().into()],
                            ))
                            .await
                            .map_err(Error::Database)?
                            .iter()
                            .map(|e| e.col::<Uuid>("id"))
                            .collect::<Result<Vec<_>>>()?;
                        txn.execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "INSERT INTO rtt_clock_events (id, pathway_id, kind, reason, event_at, created_at) \
                             VALUES ($1,$2,'stopped','validation_removal',$3,now())",
                            [Uuid::new_v4().into(), pathway.into(), stopped_at.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                        txn.execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE rtt_pathways SET status = 'stopped', stopped_at = $2, updated_at = now() \
                             WHERE id = $1 AND stopped_at IS NULL",
                            [pathway.into(), stopped_at.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                        txn.execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE validation_items SET state = 'removed', decided_by = $2, decided_at = now(), \
                                    decision_note = $3, removed_at = $4, rtt_stop_event_at = $4, removed_waitlist_ids = $5 \
                             WHERE id = $1",
                            [
                                item.into(),
                                by.clone().into(),
                                note_s.clone().into(),
                                stopped_at.into(),
                                removed.clone().into(),
                            ],
                        ))
                        .await
                        .map_err(Error::Database)?;
                        for entry in &removed {
                            AuditLogRepository::log_with(
                                txn,
                                "waitlist_entry",
                                *entry,
                                "remove",
                                None,
                                Some(serde_json::json!({ "campaign_id": campaign, "decided_by": by })),
                                Some("validation_removal"),
                                None,
                                &ctx,
                            )
                            .await?;
                        }
                        AuditLogRepository::log_with(
                            txn,
                            "rtt_pathway",
                            pathway,
                            "stop",
                            None,
                            Some(serde_json::json!({ "reason": "validation_removal" })),
                            Some("validation_removal"),
                            None,
                            &ctx,
                        )
                        .await?;
                        OutboxRepository::publish(
                            txn,
                            "ValidationRemoval",
                            &serde_json::json!({ "item_id": item, "campaign_id": campaign, "pathway_id": pathway }),
                        )
                        .await?;
                    }
                    AuditLogRepository::log_with(
                        txn,
                        "validation_item",
                        item,
                        &decision_s,
                        None,
                        Some(serde_json::json!({ "decided_by": by, "note": note_s })),
                        Some(&decision_s),
                        None,
                        &ctx,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await
            .map_err(Error::from)?;
        self.item(campaign, item).await
    }

    /// Undo a removal within [`REINSTATE_DAYS`]: the waiting-list entries go back to waiting and the clock
    /// continues from where it stopped, as if the patient had never left.
    pub async fn reinstate(
        &self,
        campaign: Uuid,
        item: Uuid,
        reinstated_by: &str,
        ctx: &UserContext,
    ) -> Result<Item> {
        let by = crate::validation::named_person(
            reinstated_by,
            "a reinstatement is made by a named person (reinstated_by)",
        )?
        .to_string();
        let ctx = ctx.clone();
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    let r = txn
                        .query_one_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT state, pathway_id, rtt_stop_event_at, removed_waitlist_ids, \
                                    removed_at > now() - make_interval(days => $3::int) AS in_time \
                             FROM validation_items WHERE id = $1 AND campaign_id = $2 FOR UPDATE",
                            [item.into(), campaign.into(), (REINSTATE_DAYS as i32).into()],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .ok_or_else(|| Error::not_found(format!("item {item} in campaign {campaign}")))?;
                    let state: String = r.col("state")?;
                    if state != "removed" {
                        return Err(Error::conflict(format!(
                            "only a removed item can be reinstated; this one is {state}"
                        )));
                    }
                    let in_time: Option<bool> = r.col("in_time")?;
                    if in_time != Some(true) {
                        return Err(Error::conflict(format!(
                            "a removal can be undone for {REINSTATE_DAYS} days; this one is older: add the patient again as a new referral"
                        )));
                    }
                    let pathway: Uuid = r.col("pathway_id")?;
                    let stopped: DateTime<Utc> = r.col("rtt_stop_event_at")?;
                    let entries: Vec<Uuid> = r.col("removed_waitlist_ids")?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE waitlist_entries SET status = 'waiting', updated_at = now() \
                         WHERE id = ANY($1) AND status = 'removed'",
                        [entries.clone().into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    // The clock resumes at the instant it stopped, so the time away is not subtracted.
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "INSERT INTO rtt_clock_events (id, pathway_id, kind, reason, event_at, created_at) \
                         VALUES ($1,$2,'resumed','validation_reinstated',$3::timestamptz + interval '1 microsecond',now())",
                        [Uuid::new_v4().into(), pathway.into(), stopped.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE rtt_pathways SET status = 'active', stopped_at = NULL, updated_at = now() WHERE id = $1",
                        [pathway.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE validation_items SET state = 'reinstated', reinstated_by = $2, reinstated_at = now() WHERE id = $1",
                        [item.into(), by.clone().into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    AuditLogRepository::log_with(
                        txn,
                        "validation_item",
                        item,
                        "reinstated",
                        None,
                        Some(serde_json::json!({ "reinstated_by": by })),
                        Some("validation_reinstated"),
                        None,
                        &ctx,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "ValidationReinstated",
                        &serde_json::json!({ "item_id": item, "pathway_id": pathway }),
                    )
                    .await?;
                    Ok(())
                })
            })
            .await
            .map_err(Error::from)?;
        self.item(campaign, item).await
    }
}
