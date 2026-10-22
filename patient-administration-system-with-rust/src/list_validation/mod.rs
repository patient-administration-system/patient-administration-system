//! Waiting-list validation campaigns (NEW-2).
//!
//! A campaign takes the pathways that have waited longest and asks, one by one, whether the care is still
//! needed (the CAP-8 "still needed?" check, made in bulk). The rules, each tested:
//!
//! - **Nothing removes anyone by itself.** A removal is a named person's decision, taken after the patient
//!   said the care is no longer needed, or after they were asked, did not answer, were sent a letter and the
//!   wait for a reply to it ran out. The code will not let that step be skipped.
//! - **A patient who said they still need it, or are unsure, is never removed** by the campaign.
//! - **Urgent pathways are never in a campaign.** A service listed for human follow-up
//!   ([`crate::follow_up`]) is left out of the cohort and refused at the decision.
//! - **A removal is recorded as one** (state `removed`, the clock stopped with the reason
//!   `validation_removal`), so it is reported apart from treatment, and it can be **reinstated** within
//!   [`REINSTATE_DAYS`]: the waiting-list entry goes back and the clock continues as if nothing had happened.
//!
//! Nothing here contacts the patient: until a carrier exists (NOT-3) a person asks, by phone or text, and
//! records it. The letter is posted by a person and recorded.

use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::outbox::OutboxRepository;
use crate::db::row::RowExt;
use crate::waits::WaitsService;
use crate::{Error, Result};

/// How long after a removal it can be undone.
pub const REINSTATE_DAYS: i64 = 90;
/// The most pathways one campaign may take.
pub const MAX_COHORT: i64 = 500;
pub const ANSWERS: [&str; 3] = ["still_needed", "no_longer_needed", "unsure"];
pub const CHANNELS: [&str; 5] = ["phone", "sms", "whatsapp", "email", "in_person"];

#[derive(Debug, Clone, Serialize)]
pub struct Campaign {
    pub id: Uuid,
    pub name: String,
    pub service_type: Option<String>,
    pub min_weeks: i32,
    pub started_by: String,
    pub reply_by: NaiveDate,
    pub final_wait_days: i32,
    pub status: String,
    pub created_at: DateTime<Utc>,
    /// Items by state (`selected`, `asked`, ...), for the campaign page.
    pub counts: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub pathway_id: Uuid,
    pub person_id: Uuid,
    pub target_service: String,
    pub state: String,
    pub answer: Option<String>,
    pub asked_at: Option<DateTime<Utc>>,
    pub letter_sent_at: Option<DateTime<Utc>>,
    pub final_by: Option<NaiveDate>,
    pub decided_by: Option<String>,
    pub removed_at: Option<DateTime<Utc>>,
    /// Why a person may act now, if they may: `letter_due`, `decision_due`.
    pub next: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewCampaign {
    pub name: String,
    pub started_by: String,
    pub service_type: Option<String>,
    /// Defaults to `PAS_CHECK_WEEKS`.
    pub min_weeks: Option<i64>,
    /// Days from now until the first reply is expected (default 14).
    pub reply_in_days: i64,
    /// Days to wait for a reply to the letter (default 14).
    pub final_wait_days: i64,
    /// How many pathways to take, longest-waiting first (1 to [`MAX_COHORT`]).
    pub limit: i64,
}

pub struct ListValidationService {
    db: DatabaseConnection,
    waits: std::sync::Arc<WaitsService>,
}

const ITEM_COLS: &str = "i.id, i.campaign_id, i.pathway_id, i.person_id, i.target_service, i.state, i.answer, \
    i.asked_at, i.letter_sent_at, i.final_by, i.decided_by, i.removed_at, \
    CASE WHEN i.state IN ('selected','asked') AND current_date > c.reply_by THEN 'letter_due' \
         WHEN i.state = 'answered' AND i.answer = 'no_longer_needed' THEN 'decision_due' \
         WHEN i.state = 'letter_sent' AND current_date > i.final_by THEN 'decision_due' END AS next";

fn row_to_item(r: &sea_orm::QueryResult) -> Result<Item> {
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
    pub fn new(db: DatabaseConnection, waits: std::sync::Arc<WaitsService>) -> Self {
        Self { db, waits }
    }

    /// Start a campaign: pick the cohort and record it. Returns the campaign with its counts.
    pub async fn start(&self, n: NewCampaign, ctx: &UserContext) -> Result<Campaign> {
        let name = n.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 200 {
            return Err(Error::validation("name must be 1 to 200 characters"));
        }
        let started_by = crate::validation::named_person(
            &n.started_by,
            "a campaign is started by a named person (started_by)",
        )?
        .to_string();
        let service = match n.service_type.as_deref().map(str::trim) {
            Some("") => return Err(Error::validation("service_type must not be empty")),
            Some(s) if s.len() > 200 => {
                return Err(Error::validation(
                    "service_type must be at most 200 characters",
                ));
            }
            Some(s) => Some(s.to_string()),
            None => None,
        };
        if !(1..=MAX_COHORT).contains(&n.limit) {
            return Err(Error::validation(format!(
                "limit must be between 1 and {MAX_COHORT}"
            )));
        }
        if !(0..=365).contains(&n.reply_in_days) || !(1..=90).contains(&n.final_wait_days) {
            return Err(Error::validation(
                "reply_in_days must be 0 to 365 and final_wait_days 1 to 90",
            ));
        }
        let min_weeks = n
            .min_weeks
            .unwrap_or_else(|| crate::config::env::int("PAS_CHECK_WEEKS"));
        if !(1..=520).contains(&min_weeks) {
            return Err(Error::validation("min_weeks must be between 1 and 520"));
        }
        // Longest-waiting first, the same cohort the CAP-8 "still needed?" list shows.
        let due = self.waits.due_for_check(Some(min_weeks), None).await?;
        let urgent = self.urgent_services().await?;
        let busy = self.pathways_in_a_live_item().await?;
        let cohort: Vec<(Uuid, Uuid, String)> = due
            .into_iter()
            .filter(|(_, p)| service.as_ref().is_none_or(|s| *s == p.target_service))
            .filter(|(_, p)| !urgent.contains(&p.target_service))
            .filter(|(_, p)| !busy.contains(&p.pathway_id))
            .take(usize::try_from(n.limit).unwrap_or(0))
            .map(|(person, p)| (person, p.pathway_id, p.target_service))
            .collect();
        if cohort.is_empty() {
            return Err(Error::conflict(
                "no pathway fits: none has waited long enough, or all are urgent, already in a campaign or checked lately",
            ));
        }
        let id = Uuid::new_v4();
        let (reply_in, final_wait) = (n.reply_in_days as i32, n.final_wait_days as i32);
        let ctx = ctx.clone();
        let (name2, by2, service2) = (name.clone(), started_by.clone(), service.clone());
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "INSERT INTO validation_campaigns (id, name, service_type, min_weeks, started_by, reply_by, final_wait_days) \
                         VALUES ($1,$2,$3,$4,$5, current_date + $6::int, $7)",
                        [
                            id.into(),
                            name2.into(),
                            service2.into(),
                            (min_weeks as i32).into(),
                            by2.clone().into(),
                            reply_in.into(),
                            final_wait.into(),
                        ],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    for (person, pathway, svc) in cohort {
                        txn.execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "INSERT INTO validation_items (id, campaign_id, pathway_id, person_id, target_service) \
                             VALUES ($1,$2,$3,$4,$5)",
                            [
                                Uuid::new_v4().into(),
                                id.into(),
                                pathway.into(),
                                person.into(),
                                svc.into(),
                            ],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    }
                    AuditLogRepository::log(
                        txn,
                        "validation_campaign",
                        id,
                        "started",
                        None,
                        Some(serde_json::json!({ "started_by": by2 })),
                        &ctx,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "ValidationCampaignStarted",
                        &serde_json::json!({ "campaign_id": id }),
                    )
                    .await?;
                    Ok(())
                })
            })
            .await
            .map_err(Error::from)?;
        self.campaign(id).await
    }

    async fn urgent_services(&self) -> Result<Vec<String>> {
        self.db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT service_type FROM follow_up_policies".to_string(),
            ))
            .await
            .map_err(Error::Database)?
            .iter()
            .map(|r| r.col::<String>("service_type"))
            .collect()
    }

    async fn pathways_in_a_live_item(&self) -> Result<Vec<Uuid>> {
        self.db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT pathway_id FROM validation_items WHERE state IN ('selected','asked','answered','letter_sent')"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?
            .iter()
            .map(|r| r.col::<Uuid>("pathway_id"))
            .collect()
    }

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

    async fn item(&self, campaign: Uuid, item: Uuid) -> Result<Item> {
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
