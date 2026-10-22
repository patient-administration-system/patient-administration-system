//! Part of [`super`]: review.

use super::*;

impl NoShowService {
    pub(super) async fn latest_review(
        &self,
        kind: &str,
        valid_days: i64,
    ) -> Result<Option<String>> {
        Ok(self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT outcome, reviewed_at > now() - make_interval(days => $3::int) AS fresh FROM noshow_reviews \
                 WHERE kind = $1 AND model_version = $2 ORDER BY reviewed_at DESC LIMIT 1",
                [kind.into(), MODEL_VERSION.into(), (valid_days as i32).into()],
            ))
            .await
            .map_err(Error::Database)?
            .and_then(|r| {
                let outcome: String = r.try_get("", "outcome").ok()?;
                let fresh: bool = r.try_get("", "fresh").ok()?;
                (outcome == "approved" && fresh).then_some(outcome)
            }))
    }

    pub async fn status(&self) -> Result<Status> {
        let switched_on = Self::switched_on();
        let fairness = self
            .latest_review("fairness", FAIRNESS_VALID_DAYS)
            .await?
            .is_some();
        let safety = self
            .latest_review("clinical_safety", SAFETY_VALID_DAYS)
            .await?
            .is_some();
        let active = switched_on && fairness && safety;
        let reason = (!active).then(|| {
            let mut missing = Vec::new();
            if !switched_on {
                missing.push("PAS_NOSHOW_SCORE is not true".to_string());
            }
            if !fairness {
                missing.push(format!(
                    "no current approved fairness review (valid {FAIRNESS_VALID_DAYS} days)"
                ));
            }
            if !safety {
                missing.push(format!(
                    "no current approved clinical safety review (valid {SAFETY_VALID_DAYS} days)"
                ));
            }
            missing.join("; ")
        });
        Ok(Status {
            model_version: MODEL_VERSION,
            switched_on,
            fairness_approved: fairness,
            clinical_safety_approved: safety,
            active,
            reason,
            medium_threshold: medium_threshold(),
            call_capacity: call_capacity(),
            call_horizon_days: call_horizon_days(),
            percentage_shown: percent_shown(),
            age_in_use: Weights::from_env().age.values().any(|v| *v != 0.0),
        })
    }

    pub(super) async fn require_active(&self) -> Result<()> {
        let s = self.status().await?;
        if s.active {
            Ok(())
        } else {
            Err(Error::conflict(format!(
                "the no-show probability is switched off: {}",
                s.reason.unwrap_or_default()
            )))
        }
    }

    /// Record a person's review of this model version. An approval needs notes saying what was looked at.
    pub async fn record_review(
        &self,
        kind: &str,
        reviewer: &str,
        outcome: &str,
        notes: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        if !["fairness", "clinical_safety"].contains(&kind) {
            return Err(Error::validation(
                "kind must be fairness or clinical_safety",
            ));
        }
        if !["approved", "rejected"].contains(&outcome) {
            return Err(Error::validation("outcome must be approved or rejected"));
        }
        let notes = notes.trim();
        let reviewer = crate::validation::named_person(
            reviewer,
            "a review is made by a named person (reviewer)",
        )?;
        if outcome == "approved" && notes.chars().count() < 40 {
            return Err(Error::validation(
                "an approval needs notes (at least 40 characters) saying what was looked at and why it is acceptable",
            ));
        }
        if outcome == "approved"
            && kind == "fairness"
            && Weights::from_env().age.values().any(|v| *v != 0.0)
            && !notes
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == "age" || w == "ages" || w == "aged")
        {
            return Err(Error::validation(
                "age bands carry weight in this configuration: a fairness approval must say why using age is justified (the notes must discuss age)",
            ));
        }
        if notes.is_empty() || notes.chars().count() > 1000 {
            return Err(Error::validation("notes must be 1 to 1000 characters"));
        }
        let id = Uuid::new_v4();
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO noshow_reviews (id, kind, reviewer, outcome, notes, model_version) VALUES ($1,$2,$3,$4,$5,$6)",
                [id.into(), kind.into(), reviewer.into(), outcome.into(), notes.into(), MODEL_VERSION.into()],
            ))
            .await
            .map_err(Error::Database)?;
        AuditLogRepository::log_with(
            &self.db,
            "noshow_model",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, MODEL_VERSION.as_bytes()),
            &format!("review_{outcome}"),
            None,
            Some(serde_json::json!({ "kind": kind, "reviewer": reviewer })),
            Some(kind),
            None,
            ctx,
        )
        .await
    }
}
