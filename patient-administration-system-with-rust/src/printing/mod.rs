//! The postal letter print queue (NOT-7).
//!
//! `print` letters wait in a queue. A person exports a batch for the mail house, which makes a batch of the
//! oldest queued letters that have an address; the mail house prints and posts them; a person then marks
//! the batch sent, which flips exactly the letters in it to `sent`.
//!
//! - **The address is not stored.** It is read from person-service when the batch is exported and goes only
//!   into the export. A letter whose patient has no address on record stays in the queue and is reported
//!   as skipped, so it is never silently lost or marked sent.
//! - **A letter is in at most one batch**, so two people exporting at once get different letters.
//! - **Marking sent is once, by a named person**, and flips exactly the batch's letters.
//! - **A patient who has opted out of nothing here:** postal letters are not a text-style channel, so
//!   STOP does not apply; a patient's choice of channel (CAP-4, CAS-10) decides whether one is made at all.

use crate::db::row::RowExt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::upstream::{PersonClient, UserContext as UpstreamUserContext};
use crate::{Error, Result};

pub const MAX_BATCH: i64 = 500;

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct QueuedLetter {
    pub letter_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ExportRow {
    pub reference: Uuid,
    pub name: String,
    pub address: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Batch {
    pub batch_id: Uuid,
    pub rows: Vec<ExportRow>,
    /// Queued letters left out because the patient has no postal address on record.
    pub skipped_no_address: usize,
}

impl Batch {
    pub fn to_csv(&self) -> String {
        let mut out = String::from("reference,name,address,subject,body\n");
        for r in &self.rows {
            out.push_str(&format!(
                "{},{},{},{},{}\n",
                r.reference,
                csv_field(&r.name),
                csv_field(&r.address),
                csv_field(&r.subject),
                csv_field(&r.body)
            ));
        }
        out
    }
}

fn csv_field(v: &str) -> String {
    let v = if v.starts_with(['=', '+', '-', '@']) {
        format!("'{v}")
    } else {
        v.to_string()
    };
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

fn display_name(p: &crate::upstream::Person) -> String {
    let n = match p.data.get("name") {
        Some(serde_json::Value::Array(a)) => a.first().cloned(),
        other => other.cloned(),
    };
    let Some(n) = n else { return String::new() };
    let given = match n.get("given") {
        Some(serde_json::Value::Array(g)) => g
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    let family = n.get("family").and_then(|v| v.as_str()).unwrap_or("");
    format!("{given} {family}").trim().to_string()
}

pub struct PrintService {
    db: DatabaseConnection,
    person_client: Option<Arc<dyn PersonClient>>,
}

impl PrintService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            person_client: None,
        }
    }

    pub fn with_person_client(&self, p: Arc<dyn PersonClient>) -> Self {
        Self {
            db: self.db.clone(),
            person_client: Some(p),
        }
    }

    /// Queued letters: print, pending, in no batch, oldest first.
    pub async fn queue(&self, limit: i64) -> Result<(i64, Vec<QueuedLetter>)> {
        let total = self
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM generated_letters g WHERE g.channel = 'print' AND g.status = 'pending' \
                 AND NOT EXISTS (SELECT 1 FROM print_batch_letters b WHERE b.letter_id = g.id)"
                    .to_string(),
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::internal("count returned no row"))?
            .col::<i64>("n")?;
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT g.id, g.created_at, g.rendered_subject FROM generated_letters g \
                 WHERE g.channel = 'print' AND g.status = 'pending' \
                   AND NOT EXISTS (SELECT 1 FROM print_batch_letters b WHERE b.letter_id = g.id) \
                 ORDER BY g.created_at LIMIT $1",
                [limit.clamp(1, 500).into()],
            ))
            .await
            .map_err(Error::Database)?;
        let items = rows
            .iter()
            .map(|r| {
                Ok(QueuedLetter {
                    letter_id: r.col("id")?,
                    created_at: r.col("created_at")?,
                    subject: r.col("rendered_subject")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((total, items))
    }

    /// Make a batch of up to `limit` queued letters that have a postal address, and return the export.
    pub async fn export_batch(
        &self,
        limit: i64,
        created_by: &str,
        ctx: &UserContext,
    ) -> Result<Batch> {
        let created_by =
            crate::validation::named_person(created_by, "a batch names who made it (created_by)")?;
        let people = self.person_client.as_ref().ok_or_else(|| {
            Error::upstream("print export needs person-service for postal addresses")
        })?;
        let limit = limit.clamp(1, MAX_BATCH);
        let upstream = UpstreamUserContext {
            user_id: ctx.user_id.clone(),
            user_ip: ctx.user_ip.clone(),
            user_agent: ctx.user_agent.clone(),
        };
        let batch_id = Uuid::new_v4();
        let created_by = created_by.to_string();
        let ctx2 = ctx.clone();
        let people = people.clone();
        let (rows, skipped) = self
            .db
            .transaction::<_, (Vec<ExportRow>, usize), Error>(|txn| {
                Box::pin(async move {
                    // Exports take turns. Without this, a second export whose snapshot predates the first one's
                    // commit can lock the same letters (the lock is gone by then) and trip the unique constraint
                    // on `print_batch_letters`. The lock is held until this transaction ends, and the query below
                    // takes its snapshot after it.
                    txn.execute_raw(Statement::from_string(
                        DbBackend::Postgres,
                        "SELECT pg_advisory_xact_lock(7011)".to_string(),
                    ))
                    .await
                    .map_err(Error::Database)?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "INSERT INTO print_batches (id, created_by) VALUES ($1,$2)",
                        [batch_id.into(), created_by.clone().into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    // Lock the oldest queued letters, skipping any another export has locked.
                    let letters = txn
                        .query_all_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT g.id, g.person_id, g.rendered_subject, g.rendered_body FROM generated_letters g \
                             WHERE g.channel = 'print' AND g.status = 'pending' \
                               AND NOT EXISTS (SELECT 1 FROM print_batch_letters b WHERE b.letter_id = g.id) \
                             ORDER BY g.created_at LIMIT $1 FOR UPDATE SKIP LOCKED",
                            [limit.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    let (mut rows, mut skipped) = (Vec::new(), 0usize);
                    for l in letters {
                        let id: Uuid = l.col("id")?;
                        let person: Uuid = l.col("person_id")?;
                        let person = people.get(person, &upstream).await;
                        let address = person.as_ref().ok().and_then(crate::transport::pickup_address);
                        let Some(address) = address else {
                            skipped += 1;
                            continue;
                        };
                        txn.execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "INSERT INTO print_batch_letters (batch_id, letter_id) VALUES ($1,$2)",
                            [batch_id.into(), id.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                        rows.push(ExportRow {
                            reference: id,
                            name: person.as_ref().map(display_name).unwrap_or_default(),
                            address,
                            subject: l.col("rendered_subject")?,
                            body: l.col("rendered_body")?,
                        });
                    }
                    if rows.is_empty() {
                        return Err(Error::conflict(
                            "there are no queued letters with a postal address to export",
                        ));
                    }
                    AuditLogRepository::log(
                        txn,
                        "print_batch",
                        batch_id,
                        "exported",
                        None,
                        Some(serde_json::json!({ "letters": rows.len(), "skipped_no_address": skipped })),
                        &ctx2,
                    )
                    .await?;
                    Ok((rows, skipped))
                })
            })
            .await
            .map_err(Error::from)?;
        Ok(Batch {
            batch_id,
            rows,
            skipped_no_address: skipped,
        })
    }

    /// The mail house has the batch: flip exactly its letters to sent. Once, by a named person.
    pub async fn mark_sent(
        &self,
        batch_id: Uuid,
        sent_by: &str,
        ctx: &UserContext,
    ) -> Result<usize> {
        let sent_by = crate::validation::named_person(
            sent_by,
            "a batch is marked sent by a named person (sent_by)",
        )?
        .to_string();
        let ctx = ctx.clone();
        self.db
            .transaction::<_, usize, Error>(|txn| {
                Box::pin(async move {
                    let claimed = txn
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE print_batches SET sent_at = now(), sent_by = $2 WHERE id = $1 AND sent_at IS NULL",
                            [batch_id.into(), sent_by.clone().into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    if claimed.rows_affected() == 0 {
                        let exists = txn
                            .query_one_raw(Statement::from_sql_and_values(
                                DbBackend::Postgres,
                                "SELECT 1 AS ok FROM print_batches WHERE id = $1",
                                [batch_id.into()],
                            ))
                            .await
                            .map_err(Error::Database)?;
                        return Err(if exists.is_some() {
                            Error::conflict("that batch has already been marked sent")
                        } else {
                            Error::not_found(format!("print batch {batch_id}"))
                        });
                    }
                    let n = txn
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE generated_letters SET status = 'sent', sent_at = now(), updated_at = now() \
                             WHERE status = 'pending' AND id IN (SELECT letter_id FROM print_batch_letters WHERE batch_id = $1)",
                            [batch_id.into()],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .rows_affected() as usize;
                    AuditLogRepository::log(
                        txn,
                        "print_batch",
                        batch_id,
                        "sent",
                        None,
                        Some(serde_json::json!({ "letters": n, "sent_by": sent_by })),
                        &ctx,
                    )
                    .await?;
                    Ok(n)
                })
            })
            .await
            .map_err(Error::from)
    }
}
