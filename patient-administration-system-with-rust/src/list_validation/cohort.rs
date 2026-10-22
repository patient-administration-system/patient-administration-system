//! Part of [`super`]: cohort.

use super::*;

impl ListValidationService {
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

    pub(super) async fn urgent_services(&self) -> Result<Vec<String>> {
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

    pub(super) async fn pathways_in_a_live_item(&self) -> Result<Vec<Uuid>> {
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
}
