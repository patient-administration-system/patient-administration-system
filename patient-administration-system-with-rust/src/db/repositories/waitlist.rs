//! waitlist repository

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::waitlist_entry;
use crate::models::waitlist::{Priority, WaitlistEntry, WaitlistStatus};
use crate::{Error, Result};

pub struct WaitlistRepository;

impl WaitlistRepository {
    pub async fn create_entry<C: Conn>(conn: &C, e: &WaitlistEntry) -> Result<WaitlistEntry> {
        let am = to_row(e);
        am.insert(conn).await?;
        Ok(e.clone())
    }

    /// Lock the entry's row for the rest of the transaction and check it is the version the caller read
    /// (OFH-1); `None` (the caller sent `*`) takes the lock and skips the comparison.
    pub async fn lock_entry_if_current<C: Conn>(
        conn: &C,
        id: Uuid,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<WaitlistEntry> {
        let m = conn
            .one_as::<waitlist_entry::Model>(Stmt::new(
                "SELECT * FROM waitlist_entries WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("waitlist entry {id}")))?;
        let entry = from_model(m)?;
        if !crate::api::rest::concurrency::is_current(entry.updated_at, expected) {
            return Err(Error::precondition_failed(
                "the waiting-list entry has changed since it was read; read it again",
            ));
        }
        Ok(entry)
    }

    pub async fn find_entry_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<WaitlistEntry>> {
        let m = conn
            .one_as::<waitlist_entry::Model>(Stmt::new(
                "SELECT * FROM waitlist_entries WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    pub async fn update_entry<C: Conn>(conn: &C, e: &WaitlistEntry) -> Result<WaitlistEntry> {
        to_row(e).update_row(conn).await?;
        Ok(e.clone())
    }

    pub async fn list_by_service<C: Conn>(conn: &C, service: &str) -> Result<Vec<WaitlistEntry>> {
        // Longest on the list first. Without an order Postgres returns rows in the order they happen to lie on
        // disk, and an UPDATE moves a row.
        let rows = conn
            .all_as::<waitlist_entry::Model>(Stmt::new(
                "SELECT * FROM waitlist_entries WHERE target_service = $1 ORDER BY created_at, id",
                [service.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    pub async fn list_by_priority<C: Conn>(
        conn: &C,
        priority: Priority,
    ) -> Result<Vec<WaitlistEntry>> {
        let rows = conn
            .all_as::<waitlist_entry::Model>(Stmt::new(
                "SELECT * FROM waitlist_entries WHERE priority = $1",
                [priority_to_str(priority).into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Returns all waitlist entries whose corresponding RTT pathways are
    /// in breach. Implementation note: v0.1 delegates the RTT computation
    /// to `RTTRepository::is_pathway_breaching` once available; here we
    /// just return the empty list as a v0.1 placeholder. Real
    /// implementation joins waitlist_entries → rtt_pathways → events.
    pub async fn list_breaches<C: Conn>(
        _conn: &C,
        _now: chrono::DateTime<chrono::Utc>,
        _threshold_weeks: u32,
    ) -> Result<Vec<WaitlistEntry>> {
        // Joining across waitlist_entries → rtt_pathways → rtt_clock_events
        // requires a multi-step query; the higher-level `waitlist` service
        // (Layer 3) composes the two repos. The repository alone returns
        // empty.
        Ok(Vec::new())
    }
}

// --- conversion helpers ---

pub(crate) fn priority_to_str(p: Priority) -> &'static str {
    match p {
        Priority::Routine => "routine",
        Priority::Urgent => "urgent",
        Priority::TwoWeekWait => "two_week_wait",
        Priority::Emergency => "emergency",
    }
}

pub(crate) fn priority_from_str(s: &str) -> Result<Priority> {
    match s {
        "routine" => Ok(Priority::Routine),
        "urgent" => Ok(Priority::Urgent),
        "two_week_wait" => Ok(Priority::TwoWeekWait),
        "emergency" => Ok(Priority::Emergency),
        other => Err(Error::internal(format!("unknown priority: {other}"))),
    }
}

pub(crate) fn waitlist_status_to_str(s: WaitlistStatus) -> &'static str {
    match s {
        WaitlistStatus::Waiting => "waiting",
        WaitlistStatus::Booked => "booked",
        WaitlistStatus::Removed => "removed",
        WaitlistStatus::Treated => "treated",
    }
}

pub(crate) fn waitlist_status_from_str(s: &str) -> Result<WaitlistStatus> {
    match s {
        "waiting" => Ok(WaitlistStatus::Waiting),
        "booked" => Ok(WaitlistStatus::Booked),
        "removed" => Ok(WaitlistStatus::Removed),
        "treated" => Ok(WaitlistStatus::Treated),
        other => Err(Error::internal(format!("unknown waitlist status: {other}"))),
    }
}

fn to_row(e: &WaitlistEntry) -> waitlist_entry::Model {
    waitlist_entry::Model {
        id: e.id,
        referral_id: e.referral_id,
        person_id: e.person_id,
        target_service: e.target_service.clone(),
        priority: priority_to_str(e.priority).to_string(),
        status: waitlist_status_to_str(e.status).to_string(),
        created_at: e.created_at.fixed_offset(),
        updated_at: e.updated_at.fixed_offset(),
    }
}

fn from_model(m: waitlist_entry::Model) -> Result<WaitlistEntry> {
    Ok(WaitlistEntry {
        id: m.id,
        referral_id: m.referral_id,
        person_id: m.person_id,
        target_service: m.target_service,
        priority: priority_from_str(&m.priority)?,
        status: waitlist_status_from_str(&m.status)?,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_waitlist_entry_roundtrip_via_row() {
        let e = WaitlistEntry::new(Uuid::new_v4(), "cardiology", Priority::Urgent);
        let am = to_row(&e);
        let m = waitlist_entry::Model {
            id: am.id,
            referral_id: am.referral_id,
            person_id: am.person_id,
            target_service: am.target_service.clone(),
            priority: am.priority.clone(),
            status: am.status.clone(),
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, e.id);
        assert_eq!(back.priority, Priority::Urgent);
        assert_eq!(back.status, WaitlistStatus::Waiting);
        assert_eq!(back.target_service, "cardiology");
    }
}
