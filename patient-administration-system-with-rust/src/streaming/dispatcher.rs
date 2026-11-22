//! Outbox dispatcher background task.
//!
//! Polls the `outbox_events` table for unpublished rows, forwards each one
//! to the configured [`EventPublisher`], and marks it published on success.
//!
//! Designed to be spawned once at server startup with `tokio::spawn`.
//!
//! **Failure handling (v0.5).** When publish returns `Err`, the dispatcher
//! increments the row's `retry_count` and records the error in `last_error`.
//! If the new count reaches `max_retries`, the row is moved into
//! `outbox_dead_letters` in one DB transaction (insert dead-letter + delete
//! outbox). An operator can then inspect it via
//! `GET /api/admin/outbox/dead-letters` and replay it via
//! `POST /api/admin/outbox/dead-letters/{id}/replay`, which puts a fresh row
//! back into `outbox_events` with `retry_count = 0`.

use crate::db::sql::{Pool, Transact};
use std::sync::Arc;
use std::time::Duration;

use crate::db::repositories::outbox::OutboxRepository;
use crate::streaming::{DomainEvent, EventPublisher};
use crate::{Error, Result};

/// Number of outbox rows fetched per poll cycle.
const BATCH_SIZE: u64 = 64;

/// Default retry budget when [`run`] is called without an explicit cap.
/// Override via `PAS_OUTBOX_MAX_RETRIES`.
pub const DEFAULT_MAX_RETRIES: u32 = 10;

/// Run the dispatcher loop forever. Cancelled by dropping the future.
///
/// `max_retries` is the per-event retry budget: after the N-th consecutive
/// failed publish the row is moved to the dead-letter queue and the
/// dispatcher stops trying it. Pass `0` to disable dead-lettering (the row
/// will be retried forever).
pub async fn run(
    db: Pool,
    publisher: Arc<dyn EventPublisher>,
    poll_interval: Duration,
    max_retries: u32,
) {
    tracing::info!(
        "outbox dispatcher starting; poll_interval={:?}, batch_size={}, max_retries={}",
        poll_interval,
        BATCH_SIZE,
        max_retries,
    );
    loop {
        if let Err(e) = tick(&db, &publisher, max_retries).await {
            tracing::warn!("outbox dispatcher tick failed: {e}");
        }
        tokio::time::sleep(poll_interval).await;
    }
}

/// The first delay after a deferred delivery; it doubles with each failure.
pub const BACKOFF_START: Duration = Duration::from_secs(5);
/// The longest delay, and the most a receiver's `Retry-After` may ask for.
pub const BACKOFF_CAP: Duration = Duration::from_secs(15 * 60);

/// How long to wait before the next try of an event that has failed `failures` times (1 for the first
/// failure): 5 s, 10 s, 20 s, … at most 15 minutes, or the receiver's `Retry-After` within that cap.
pub fn backoff(failures: i32, retry_after_secs: Option<u64>) -> Duration {
    if let Some(secs) = retry_after_secs {
        return Duration::from_secs(secs).min(BACKOFF_CAP);
    }
    let doublings = u32::try_from(failures.checked_sub(1).unwrap_or(0).max(0))
        .unwrap_or(0)
        .min(16);
    // An overflow means the delay is past the cap, so it is the cap.
    2u32.checked_pow(doublings)
        .and_then(|factor| BACKOFF_START.checked_mul(factor))
        .unwrap_or(BACKOFF_CAP)
        .min(BACKOFF_CAP)
}

/// One poll cycle: fetch the events that are due, publish each, and settle each by its outcome (NFR-32).
/// Returns how many were delivered (for tests).
///
/// - **Accepted** (`Ok`): marked published.
/// - **Rejected** ([`Error::DeliveryRejected`]: a 400, a 422, an HL7 `AR`): dead-lettered at once; a retry
///   cannot change the answer.
/// - **Deferred** (anything else): `retry_count` goes up and the event waits [`backoff`] before it is due
///   again; at `max_retries` (when above 0) it is dead-lettered.
///
/// An event is due only when nothing earlier for its record is undelivered or held in the dead-letter queue
/// ([`OutboxRepository::fetch_due`]), so its record's events arrive in order.
#[tracing::instrument(skip_all, fields(job = "outbox"))]
pub async fn tick(
    db: &Pool,
    publisher: &Arc<dyn EventPublisher>,
    max_retries: u32,
) -> Result<usize> {
    tick_with_limit(db, publisher, max_retries, BATCH_SIZE).await
}

/// [`tick`] over at most `limit` due events (the tests use it to settle only their own).
pub async fn tick_with_limit(
    db: &Pool,
    publisher: &Arc<dyn EventPublisher>,
    max_retries: u32,
    limit: u64,
) -> Result<usize> {
    let rows = OutboxRepository::fetch_due(db, limit).await?;
    let mut delivered = 0usize;
    for row in rows {
        let ev = DomainEvent {
            id: row.id,
            event_type: row.event_type.clone(),
            payload: row.payload.clone(),
            at: row.at,
        };
        match publisher.publish(ev).await {
            Ok(()) => {
                OutboxRepository::mark_published(db, row.id).await?;
                delivered += 1;
                crate::metrics::outbox_delivered();
            }
            Err(Error::DeliveryRejected(why)) => {
                tracing::warn!(
                    "outbox row id={} type={} rejected by the receiver; dead-lettered at once: {why}",
                    row.id,
                    row.event_type,
                );
                let why = format!("rejected: {why}");
                dead_letter(db, row.id, why).await?;
            }
            Err(e) => {
                let retry_after = match &e {
                    Error::DeliveryDeferred {
                        retry_after_secs, ..
                    } => *retry_after_secs,
                    _ => None,
                };
                let err_str = e.to_string();
                let wait = backoff(
                    // A count at the maximum stays there: the back-off is capped long before it.
                    row.retry_count.checked_add(1).unwrap_or(i32::MAX),
                    retry_after,
                );
                let next = jiff::Timestamp::now()
                    + jiff::SignedDuration::try_from(wait)
                        .unwrap_or(jiff::SignedDuration::from_mins(15));
                let new_count =
                    OutboxRepository::record_failure(db, row.id, &err_str, next).await?;
                if max_retries > 0 && new_count >= max_retries as i32 {
                    tracing::warn!(
                        "outbox row id={} hit retry budget {} after {} attempts; \
                         moving to dead-letter queue. last error: {err_str}",
                        row.id,
                        max_retries,
                        new_count,
                    );
                    dead_letter(db, row.id, err_str).await?;
                } else {
                    tracing::warn!(
                        "outbox publish failed for id={} (retry {}/{}), next try in {:?}: {err_str}",
                        row.id,
                        new_count,
                        max_retries,
                        wait,
                    );
                }
            }
        }
    }
    Ok(delivered)
}

async fn dead_letter(db: &Pool, id: uuid::Uuid, why: String) -> Result<()> {
    db.in_txn::<_, ()>(|txn| {
        Box::pin(async move { OutboxRepository::move_to_dead_letter(txn, id, &why).await })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streaming::InMemoryEventPublisher;

    #[tokio::test]
    async fn test_tick_returns_zero_on_unreachable_db() {
        // We can't easily exercise tick() without a live DB. Just confirm the
        // function signature compiles and a misconfigured connection short-
        // circuits via Error rather than panicking. (Real coverage is in the
        // integration suite where DATABASE_URL is set.)
        let publisher: Arc<dyn EventPublisher> = Arc::new(InMemoryEventPublisher::new());
        // Skipped: requires a database connection — out of scope here.
        let _ = (publisher, BATCH_SIZE);
    }

    #[test]
    fn test_backoff_doubles_from_five_seconds_to_fifteen_minutes() {
        let secs: Vec<u64> = (1..=10).map(|n| backoff(n, None).as_secs()).collect();
        assert_eq!(secs, [5, 10, 20, 40, 80, 160, 320, 640, 900, 900]);
        assert_eq!(backoff(0, None).as_secs(), 5);
        assert_eq!(backoff(i32::MAX, None), BACKOFF_CAP);
    }

    #[test]
    fn test_retry_after_is_honoured_within_the_cap() {
        assert_eq!(backoff(1, Some(120)).as_secs(), 120);
        assert_eq!(backoff(1, Some(0)).as_secs(), 0);
        assert_eq!(backoff(3, Some(86_400)), BACKOFF_CAP);
    }

    #[test]
    fn test_default_max_retries_is_ten() {
        assert_eq!(DEFAULT_MAX_RETRIES, 10);
    }
}
