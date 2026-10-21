//! Wording experiments (CAS-7).
//!
//! An experiment compares two or more wordings of one message (for example the second reminder). Each
//! variant is a complete template group, so a patient always gets a message in their language. The rules:
//!
//! - **Deterministic.** An appointment's variant is a function of the experiment and the appointment ids
//!   only ([`pick`]); it is assigned once and recorded, so asking again gives the same answer and the result
//!   can be read out later. Nothing about the patient (their deprivation, language, history, a risk guess)
//!   takes part in who gets which wording.
//! - **Wording only.** An experiment never changes who is contacted, when, how often, or what slot they are
//!   offered (NFR-11).
//! - **A read-out that protects people.** Results are split by deprivation quintile and language, with the
//!   analytics feed's rules: no identifier, small cells withheld, `k` never below the configured minimum.
//! - **Mind the words.** Wording about "the cost to the NHS" of a missed appointment can read as blame. Review
//!   every variant with that in mind (`governance/messaging-rules.md`).

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::analytics::{AnalyticsService, suppress};
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::letter::LetterRepository;
use crate::{Error, Result};

/// The variant for an appointment: the first eight bytes of SHA-256 over the two ids, modulo the number of
/// variants. A function of the ids and nothing else.
pub fn pick(experiment: Uuid, appointment: Uuid, variants: usize) -> usize {
    let mut h = Sha256::new();
    h.update(experiment.as_bytes());
    h.update(appointment.as_bytes());
    let d = h.finalize();
    let n = u64::from_be_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]);
    (n % variants.max(1) as u64) as usize
}

/// The template group to send for this appointment: the assigned variant's, if a live experiment covers
/// `template_name`, else `template_name` itself. Assigns (and records) on first use.
pub async fn resolve_name<C: ConnectionTrait>(
    conn: &C,
    appointment_id: Uuid,
    template_name: &str,
) -> Result<String> {
    let Some(exp) = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT id FROM message_experiments WHERE template_name = $1 AND stopped_at IS NULL",
            [template_name.into()],
        ))
        .await
        .map_err(Error::Database)?
    else {
        return Ok(template_name.to_string());
    };
    let exp_id: Uuid = exp.try_get("", "id").map_err(Error::Database)?;
    // Already assigned: the same wording as before.
    if let Some(r) = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT v.template_name FROM message_experiment_assignments a \
             JOIN message_experiment_variants v ON v.experiment_id = a.experiment_id AND v.variant = a.variant \
             WHERE a.experiment_id = $1 AND a.appointment_id = $2",
            [exp_id.into(), appointment_id.into()],
        ))
        .await
        .map_err(Error::Database)?
    {
        return r.try_get("", "template_name").map_err(Error::Database);
    }
    let variants = conn
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT variant, template_name FROM message_experiment_variants WHERE experiment_id = $1 ORDER BY variant",
            [exp_id.into()],
        ))
        .await
        .map_err(Error::Database)?;
    if variants.is_empty() {
        return Ok(template_name.to_string());
    }
    let chosen = &variants[pick(exp_id, appointment_id, variants.len())];
    let variant: String = chosen.try_get("", "variant").map_err(Error::Database)?;
    let name: String = chosen
        .try_get("", "template_name")
        .map_err(Error::Database)?;
    conn.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO message_experiment_assignments (experiment_id, appointment_id, variant) VALUES ($1,$2,$3) \
         ON CONFLICT DO NOTHING",
        [exp_id.into(), appointment_id.into(), variant.into()],
    ))
    .await
    .map_err(Error::Database)?;
    Ok(name)
}

#[derive(Debug, Clone, Serialize)]
pub struct Experiment {
    pub id: Uuid,
    pub template_name: String,
    pub variants: Vec<(String, String)>,
    pub started_by: String,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadoutCell {
    pub variant: String,
    pub deprivation_quintile: String,
    pub language: String,
    pub assigned: Option<i64>,
    pub confirmed: Option<i64>,
    pub attended: Option<i64>,
    pub did_not_attend: Option<i64>,
    /// attended / (attended + did not attend), to three places, over appointments whose outcome is known.
    pub attendance_rate: Option<f64>,
    pub suppressed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Readout {
    pub experiment_id: Uuid,
    pub k: i64,
    pub total_assigned: i64,
    pub suppressed_cells: usize,
    pub cells: Vec<ReadoutCell>,
}

pub struct ExperimentService {
    db: DatabaseConnection,
}

impl ExperimentService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Start an experiment on `template_name`. Needs two to eight variants, each an existing, complete
    /// template group (every required language active); at most one live experiment per message.
    pub async fn start(
        &self,
        template_name: &str,
        variants: Vec<(String, String)>,
        started_by: &str,
        ctx: &UserContext,
    ) -> Result<Uuid> {
        let template_name = template_name.trim();
        let started_by = started_by.trim().to_string();
        if template_name.is_empty() || started_by.is_empty() {
            return Err(Error::validation(
                "template_name and started_by (a named person) are required",
            ));
        }
        if !(2..=8).contains(&variants.len()) {
            return Err(Error::validation(
                "an experiment compares two to eight variants",
            ));
        }
        let mut keys: Vec<&str> = variants.iter().map(|(k, _)| k.as_str()).collect();
        keys.sort();
        keys.dedup();
        if keys.len() != variants.len()
            || keys
                .iter()
                .any(|k| k.len() != 1 || !k.chars().all(|c| c.is_ascii_uppercase()))
        {
            return Err(Error::validation(
                "variants are named A, B, C, ... once each",
            ));
        }
        let required = crate::communication::templates::required_locales();
        for (_, group) in &variants {
            let active = LetterRepository::list_active_by_name(&self.db, group).await?;
            let locales: Vec<String> = active.iter().map(|t| t.locale.clone()).collect();
            if active.is_empty()
                || !crate::communication::templates::missing(&required, &locales).is_empty()
            {
                return Err(Error::validation(format!(
                    "variant group {group:?} is missing or lacks an active version in every required language ({})",
                    required.join(", ")
                )));
            }
        }
        let id = Uuid::new_v4();
        let (name, ctx) = (template_name.to_string(), ctx.clone());
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    let made = txn
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "INSERT INTO message_experiments (id, template_name, started_by) VALUES ($1,$2,$3)",
                            [id.into(), name.clone().into(), started_by.clone().into()],
                        ))
                        .await;
                    if let Err(e) = made {
                        return Err(match e.sql_err() {
                            Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) => {
                                Error::conflict("that message already has a running experiment; stop it first")
                            }
                            _ => Error::Database(e),
                        });
                    }
                    for (k, g) in &variants {
                        txn.execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "INSERT INTO message_experiment_variants (experiment_id, variant, template_name) VALUES ($1,$2,$3)",
                            [id.into(), k.clone().into(), g.clone().into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    }
                    AuditLogRepository::log(
                        txn,
                        "message_experiment",
                        id,
                        "started",
                        None,
                        Some(serde_json::json!({ "template_name": name, "variants": variants.len(), "started_by": started_by })),
                        &ctx,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Connection(c) => Error::Database(c),
                sea_orm::TransactionError::Transaction(t) => t,
            })?;
        Ok(id)
    }

    /// Stop an experiment: new appointments get the ordinary wording again; those already assigned keep
    /// theirs (their later steps must not change wording mid-way) and the read-out stays available.
    pub async fn stop(&self, id: Uuid, stopped_by: &str, ctx: &UserContext) -> Result<()> {
        let by = stopped_by.trim();
        if by.is_empty() {
            return Err(Error::validation(
                "stopping is done by a named person (stopped_by)",
            ));
        }
        let r = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE message_experiments SET stopped_at = now(), stopped_by = $2 WHERE id = $1 AND stopped_at IS NULL",
                [id.into(), by.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if r.rows_affected() == 0 {
            return Err(Error::conflict(
                "that experiment is unknown or already stopped",
            ));
        }
        AuditLogRepository::log(
            &self.db,
            "message_experiment",
            id,
            "stopped",
            None,
            Some(serde_json::json!({ "stopped_by": by })),
            ctx,
        )
        .await
    }

    pub async fn list(&self) -> Result<Vec<Experiment>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT id, template_name, started_by, started_at, stopped_at FROM message_experiments ORDER BY started_at DESC LIMIT 200"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            let id: Uuid = r.try_get("", "id").map_err(Error::Database)?;
            let vs = self
                .db
                .query_all_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT variant, template_name FROM message_experiment_variants WHERE experiment_id = $1 ORDER BY variant",
                    [id.into()],
                ))
                .await
                .map_err(Error::Database)?;
            out.push(Experiment {
                id,
                template_name: r.try_get("", "template_name").map_err(Error::Database)?,
                variants: vs
                    .iter()
                    .map(|v| {
                        Ok((
                            v.try_get("", "variant").map_err(Error::Database)?,
                            v.try_get("", "template_name").map_err(Error::Database)?,
                        ))
                    })
                    .collect::<Result<_>>()?,
                started_by: r.try_get("", "started_by").map_err(Error::Database)?,
                started_at: r.try_get("", "started_at").map_err(Error::Database)?,
                stopped_at: r.try_get("", "stopped_at").map_err(Error::Database)?,
            });
        }
        Ok(out)
    }

    /// How each wording did, by deprivation quintile and language, with small cells withheld.
    pub async fn readout(&self, id: Uuid, k: Option<i64>) -> Result<Readout> {
        let exists = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT 1 AS ok FROM message_experiments WHERE id = $1",
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if exists.is_none() {
            return Err(Error::not_found(format!("experiment {id}")));
        }
        let k = k.unwrap_or(0).max(AnalyticsService::minimum_k());
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT x.variant, COALESCE(d.deprivation_quintile::text, 'unknown') AS q, \
                        COALESCE(cp.preferred_language, 'unknown') AS lang, count(*)::bigint AS n, \
                        count(*) FILTER (WHERE a.confirmed_at IS NOT NULL OR a.status IN ('arrived','fulfilled'))::bigint AS confirmed, \
                        count(*) FILTER (WHERE a.status IN ('arrived','fulfilled'))::bigint AS attended, \
                        count(*) FILTER (WHERE a.status = 'no_show')::bigint AS dna \
                 FROM message_experiment_assignments x JOIN appointments a ON a.id = x.appointment_id \
                 LEFT JOIN person_demographics d ON d.person_id = a.person_id \
                 LEFT JOIN patient_communication_profiles cp ON cp.person_id = a.person_id \
                 WHERE x.experiment_id = $1 GROUP BY 1, 2, 3 ORDER BY 1, 2, 3",
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut data = Vec::new();
        let mut extra = Vec::new();
        let mut total = 0;
        for r in &rows {
            let variant: String = r.try_get("", "variant").map_err(Error::Database)?;
            let q: String = r.try_get("", "q").map_err(Error::Database)?;
            let lang: String = r.try_get("", "lang").map_err(Error::Database)?;
            let n: i64 = r.try_get("", "n").map_err(Error::Database)?;
            total += n;
            data.push((
                vec![
                    ("variant".to_string(), variant.clone()),
                    ("deprivation_quintile".to_string(), q.clone()),
                    ("language".to_string(), lang.clone()),
                ],
                n,
            ));
            extra.push((
                r.try_get::<i64>("", "confirmed").map_err(Error::Database)?,
                r.try_get::<i64>("", "attended").map_err(Error::Database)?,
                r.try_get::<i64>("", "dna").map_err(Error::Database)?,
            ));
        }
        let flags = suppress(data, k);
        let cells: Vec<ReadoutCell> = flags
            .into_iter()
            .zip(extra)
            .map(|(c, (confirmed, attended, dna))| {
                let g = |d: &str| {
                    c.dims
                        .iter()
                        .find(|(n, _)| n == d)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default()
                };
                let resolved = attended + dna;
                match c.count {
                    Some(n) => ReadoutCell {
                        variant: g("variant"),
                        deprivation_quintile: g("deprivation_quintile"),
                        language: g("language"),
                        assigned: Some(n),
                        confirmed: Some(confirmed),
                        attended: Some(attended),
                        did_not_attend: Some(dna),
                        attendance_rate: (resolved > 0).then(|| {
                            ((attended as f64 / resolved as f64) * 1000.0).round() / 1000.0
                        }),
                        suppressed: false,
                    },
                    None => ReadoutCell {
                        variant: g("variant"),
                        deprivation_quintile: g("deprivation_quintile"),
                        language: g("language"),
                        assigned: None,
                        confirmed: None,
                        attended: None,
                        did_not_attend: None,
                        attendance_rate: None,
                        suppressed: true,
                    },
                }
            })
            .collect();
        Ok(Readout {
            experiment_id: id,
            k,
            total_assigned: total,
            suppressed_cells: cells.iter().filter(|c| c.suppressed).count(),
            cells,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_variant_is_a_function_of_the_two_ids_only() {
        let (e, a) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(pick(e, a, 2), pick(e, a, 2));
        assert_eq!(pick(e, a, 3), pick(e, a, 3));
        assert!(pick(e, a, 1) == 0);
    }

    #[test]
    fn test_the_split_is_roughly_even() {
        let e = Uuid::new_v4();
        let mut counts = [0usize; 3];
        for _ in 0..3000 {
            counts[pick(e, Uuid::new_v4(), 3)] += 1;
        }
        for c in counts {
            assert!((850..=1150).contains(&c), "{counts:?}");
        }
    }

    #[test]
    fn test_the_same_appointment_can_land_differently_in_another_experiment() {
        let a = Uuid::new_v4();
        let differ = (0..200)
            .filter(|_| pick(Uuid::new_v4(), a, 2) != pick(Uuid::new_v4(), a, 2))
            .count();
        assert!(
            differ > 20,
            "assignment must not be tied to the appointment across experiments: {differ}"
        );
    }
}
