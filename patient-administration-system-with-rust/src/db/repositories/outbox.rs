//! outbox repository — writes domain events into `outbox_events` for the
//! background dispatcher to publish, and manages the v0.5 dead-letter
//! queue (`outbox_dead_letters`) for events that exceed the retry budget.

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::{outbox_dead_letter, outbox_event};
use crate::{Error, Result};

pub struct OutboxRepository;

impl OutboxRepository {
    /// Insert a new outbox row with `published = false`, `retry_count = 0`.
    ///
    /// Call inside the same transaction as the entity write to guarantee
    /// "exactly the rows we committed produce events".
    pub async fn publish<C: Conn>(
        conn: &C,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        let am = outbox_event::Model {
            id: Uuid::new_v4(),
            event_type: event_type.to_string(),
            payload: payload.clone(),
            published: false,
            at: chrono::Utc::now().fixed_offset(),
            retry_count: 0,
            last_attempted_at: None,
            last_error: None,
        };
        am.insert(conn).await?;
        Ok(())
    }

    /// Fetch the next batch of unpublished outbox rows, oldest first.
    pub async fn fetch_unpublished<C: Conn>(
        conn: &C,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<outbox_event::Model>> {
        let rows = rows.into();
        conn.all_as::<outbox_event::Model>(Stmt::new(
            "SELECT * FROM outbox_events WHERE published = $1 ORDER BY at, id LIMIT $2 OFFSET $3",
            [false.into(), rows.limit().into(), rows.offset().into()],
        ))
        .await
    }

    /// Mark a single outbox row as published. Idempotent.
    pub async fn mark_published<C: Conn>(conn: &C, id: Uuid) -> Result<()> {
        let m = conn
            .one_as::<outbox_event::Model>(Stmt::new(
                "SELECT * FROM outbox_events WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        if let Some(m) = m {
            let mut am = m.patch();
            am.set("published", true);
            am.set("last_attempted_at", Some(chrono::Utc::now().fixed_offset()));
            am.set("last_error", None::<String>);
            am.update(conn).await?;
        }
        Ok(())
    }

    /// Record a failed publish attempt: increment `retry_count`, store the
    /// error string in `last_error`, stamp `last_attempted_at`. Returns the
    /// new retry count (so the dispatcher can decide whether to dead-letter).
    ///
    /// Returns `Ok(0)` and a no-op if the row is gone (idempotent against
    /// concurrent dead-lettering / replay).
    pub async fn record_failure<C: Conn>(conn: &C, id: Uuid, error: &str) -> Result<i32> {
        let m = match conn
            .one_as::<outbox_event::Model>(Stmt::new(
                "SELECT * FROM outbox_events WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
        {
            Some(m) => m,
            None => return Ok(0),
        };
        let new_count = m.retry_count.saturating_add(1);
        let mut am = m.patch();
        am.set("retry_count", new_count);
        am.set("last_attempted_at", Some(chrono::Utc::now().fixed_offset()));
        am.set("last_error", Some(error.to_string()));
        am.update(conn).await?;
        Ok(new_count)
    }

    /// Move an outbox row to the dead-letter queue: insert a corresponding
    /// `outbox_dead_letters` row carrying the same payload + the recorded
    /// failure metadata, then delete the original `outbox_events` row.
    /// Idempotent — returns `Ok(())` if the row is already gone.
    ///
    /// The caller should pass a `DatabaseTransaction` so the insert + delete
    /// commit atomically.
    pub async fn move_to_dead_letter<C: Conn>(conn: &C, id: Uuid, error: &str) -> Result<()> {
        let m = match conn
            .one_as::<outbox_event::Model>(Stmt::new(
                "SELECT * FROM outbox_events WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
        {
            Some(m) => m,
            None => return Ok(()),
        };
        let now = chrono::Utc::now().fixed_offset();
        let dl = outbox_dead_letter::Model {
            id: Uuid::new_v4(),
            original_id: m.id,
            event_type: m.event_type.clone(),
            payload: m.payload.clone(),
            created_at: m.at,
            dead_lettered_at: now,
            retry_count: m.retry_count,
            last_error: error.to_string(),
        };
        dl.insert(conn).await?;
        conn.run(Stmt::new(
            "DELETE FROM outbox_events WHERE id = $1",
            [id.into()],
        ))
        .await?;
        Ok(())
    }
}

/// Read + replay surface for the v0.5 dead-letter queue.
pub struct DeadLetterRepository;

impl DeadLetterRepository {
    /// Newest-first list of dead-letter rows, capped at `limit`.
    pub async fn list<C: Conn>(
        conn: &C,
        rows: impl Into<crate::paging::Rows>,
    ) -> Result<Vec<outbox_dead_letter::Model>> {
        let rows = rows.into();
        conn.all_as::<outbox_dead_letter::Model>(Stmt::new(
            "SELECT * FROM outbox_dead_letters ORDER BY dead_lettered_at DESC, id DESC LIMIT $1 OFFSET $2",
            [rows.limit().into(), rows.offset().into()],
        ))
        .await
    }

    /// Find one DLQ row by id (the DLQ id, not the `original_id`).
    pub async fn find_by_id<C: Conn>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<outbox_dead_letter::Model>> {
        conn.one_as::<outbox_dead_letter::Model>(Stmt::new(
            "SELECT * FROM outbox_dead_letters WHERE id = $1 LIMIT 1",
            [id.into()],
        ))
        .await
    }

    /// Count of DLQ rows. Cheap admin / dashboard read.
    pub async fn count<C: Conn>(conn: &C) -> Result<u64> {
        conn.count(Stmt::text("SELECT count(*) AS n FROM outbox_dead_letters"))
            .await
    }

    /// Replay a DLQ row back into `outbox_events`: insert a fresh outbox
    /// row carrying the same payload (and the original `at` so dispatcher
    /// ordering reflects the original event order, not the replay time),
    /// having deleted the DLQ row. The new row has `retry_count = 0` and
    /// `last_error = NULL` so the dispatcher gets a clean slate.
    ///
    /// Returns the new outbox row id. `Error::NotFound` if the DLQ id is
    /// already gone. The caller should pass a `DatabaseTransaction`.
    ///
    /// The dead letter is taken with `DELETE … RETURNING` before anything is written, so two replays of it at
    /// once cannot both send it: the second waits on the row's lock, finds it gone, and is a `NotFound`. A
    /// read and a later delete let both through, each queueing the event again under a new id (OFH-11).
    pub async fn replay<C: Conn>(conn: &C, id: Uuid) -> Result<Uuid> {
        let dl = conn
            .one_as::<outbox_dead_letter::Model>(Stmt::new(
                "DELETE FROM outbox_dead_letters WHERE id = $1 RETURNING *",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("outbox_dead_letter {id}")))?;
        let new_id = Uuid::new_v4();
        let am = outbox_event::Model {
            id: new_id,
            event_type: dl.event_type.clone(),
            payload: dl.payload.clone(),
            published: false,
            at: dl.created_at,
            retry_count: 0,
            last_attempted_at: None,
            last_error: None,
        };
        am.insert(conn).await?;
        Ok(new_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outbox_repository_is_zero_sized() {
        // The repo is intentionally a unit struct — no per-instance state.
        assert_eq!(std::mem::size_of::<OutboxRepository>(), 0);
        assert_eq!(std::mem::size_of::<DeadLetterRepository>(), 0);
    }

    #[test]
    fn test_payload_value_constructs_cleanly() {
        // Confirms the payload type used by `publish` is well-formed JSON.
        let v = serde_json::json!({
            "person_id": Uuid::new_v4(),
            "name": "alice",
        });
        assert!(v.is_object());
    }
}
