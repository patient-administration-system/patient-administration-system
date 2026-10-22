//! Part of [`super`]: steps.

use super::*;

impl ListValidationService {
    /// A person has asked the patient, by `channel`. `selected` becomes `asked`.
    pub async fn mark_asked(
        &self,
        campaign: Uuid,
        item: Uuid,
        asked_by: &str,
        channel: &str,
        ctx: &UserContext,
    ) -> Result<Item> {
        if !CHANNELS.contains(&channel) {
            return Err(Error::validation(format!(
                "channel must be one of {}",
                CHANNELS.join(", ")
            )));
        }
        let by = crate::validation::named_person(
            asked_by,
            "who asked the patient (asked_by) is required",
        )?;
        let cur = self.item(campaign, item).await?;
        if cur.state != "selected" {
            return Err(Error::conflict(format!(
                "only a selected item can be marked asked; this one is {}",
                cur.state
            )));
        }
        let check = Uuid::new_v4();
        let (by_s, ch, ctx) = (by.to_string(), channel.to_string(), ctx.clone());
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "INSERT INTO still_needed_checks (id, pathway_id, person_id, asked_by) \
                         SELECT $1, pathway_id, person_id, $3 FROM validation_items WHERE id = $2",
                        [check.into(), item.into(), by_s.clone().into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    let n = txn
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE validation_items SET state = 'asked', asked_by = $2, asked_channel = $3, \
                                    asked_at = now(), check_id = $4 WHERE id = $1 AND state = 'selected'",
                            [item.into(), by_s.clone().into(), ch.into(), check.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    if n.rows_affected() == 0 {
                        return Err(Error::conflict("that item was changed by someone else"));
                    }
                    AuditLogRepository::log(
                        txn,
                        "validation_item",
                        item,
                        "asked",
                        None,
                        Some(serde_json::json!({ "asked_by": by_s })),
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

    /// The patient's answer, recorded by a person. Allowed while the item is open (`selected`, `asked` or
    /// `letter_sent`); an answer after the letter counts. It changes nothing else.
    pub async fn record_answer(
        &self,
        campaign: Uuid,
        item: Uuid,
        answer: &str,
        recorded_by: &str,
        ctx: &UserContext,
    ) -> Result<Item> {
        if !ANSWERS.contains(&answer) {
            return Err(Error::validation(format!(
                "answer must be one of {}",
                ANSWERS.join(", ")
            )));
        }
        let by = crate::validation::named_person(
            recorded_by,
            "an answer is recorded by a named person (recorded_by)",
        )?
        .to_string();
        let cur = self.item(campaign, item).await?;
        if !["selected", "asked", "letter_sent"].contains(&cur.state.as_str()) {
            return Err(Error::conflict(format!(
                "this item is {}: it can no longer be answered",
                cur.state
            )));
        }
        let (answer_s, ctx) = (answer.to_string(), ctx.clone());
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    // The CAP-8 check, so "last check" on the patient's waits shows this.
                    let check = Uuid::new_v4();
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "INSERT INTO still_needed_checks (id, pathway_id, person_id, asked_by, answer, recorded_by, answered_at) \
                         SELECT $1, i.pathway_id, i.person_id, COALESCE(i.asked_by, $3), $4, $3, now() \
                         FROM validation_items i WHERE i.id = $2 AND i.check_id IS NULL",
                        [check.into(), item.into(), by.clone().into(), answer_s.clone().into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE still_needed_checks SET answer = $2, recorded_by = $3, answered_at = now() \
                         WHERE id = (SELECT check_id FROM validation_items WHERE id = $1) AND answer IS NULL",
                        [item.into(), answer_s.clone().into(), by.clone().into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    let n = txn
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE validation_items SET state = 'answered', answer = $2, answered_at = now(), \
                                    check_id = COALESCE(check_id, (SELECT id FROM still_needed_checks \
                                      WHERE pathway_id = validation_items.pathway_id ORDER BY asked_at DESC LIMIT 1)) \
                             WHERE id = $1 AND state IN ('selected','asked','letter_sent')",
                            [item.into(), answer_s.clone().into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    if n.rows_affected() == 0 {
                        return Err(Error::conflict("that item was changed by someone else"));
                    }
                    AuditLogRepository::log_with(
                        txn,
                        "validation_item",
                        item,
                        "answered",
                        None,
                        Some(serde_json::json!({ "answer": answer_s, "recorded_by": by })),
                        Some(&answer_s),
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

    /// A person has posted the letter to a patient who did not reply by the campaign's date. Starts the wait
    /// for a reply to the letter. Refused before the date: the letter is the step after being asked.
    pub async fn letter_sent(
        &self,
        campaign: Uuid,
        item: Uuid,
        sent_by: &str,
        ctx: &UserContext,
    ) -> Result<Item> {
        let by = crate::validation::named_person(
            sent_by,
            "the letter is sent by a named person (sent_by)",
        )?
        .to_string();
        let cur = self.item(campaign, item).await?;
        if cur.next.as_deref() != Some("letter_due") {
            return Err(Error::conflict(
                if ["selected", "asked"].contains(&cur.state.as_str()) {
                    "the reply date has not passed yet".to_string()
                } else {
                    format!(
                        "a letter is for an unanswered item; this one is {}",
                        cur.state
                    )
                },
            ));
        }
        let ctx = ctx.clone();
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    let n = txn
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE validation_items SET state = 'letter_sent', letter_sent_by = $2, letter_sent_at = now(), \
                                    final_by = current_date + (SELECT final_wait_days FROM validation_campaigns WHERE id = campaign_id) \
                             WHERE id = $1 AND state IN ('selected','asked')",
                            [item.into(), by.clone().into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    if n.rows_affected() == 0 {
                        return Err(Error::conflict("that item was changed by someone else"));
                    }
                    AuditLogRepository::log(
                        txn,
                        "validation_item",
                        item,
                        "letter_sent",
                        None,
                        Some(serde_json::json!({ "sent_by": by })),
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
}
