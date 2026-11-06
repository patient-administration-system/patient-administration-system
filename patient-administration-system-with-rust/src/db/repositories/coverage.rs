//! coverage repository (v0.10.0).
//!
//! `coverages` is a flat table — no JSONB columns, no joins required.
//! The repo carries the usual create / read / update / list shape plus
//! two convenience methods (`link_to_account`, `set_status`) that are
//! cheaper than a full PUT round-trip.

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::coverage;
use crate::models::coverage::{Coverage, CoverageKind, CoverageStatus};
use crate::{Error, Result};

pub struct CoverageRepository;

impl CoverageRepository {
    /// Insert a new coverage row.
    pub async fn create<C: Conn>(conn: &C, c: &Coverage) -> Result<Coverage> {
        let am = to_row(c);
        am.insert(conn).await?;
        Ok(c.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Coverage>> {
        let m = conn
            .one_as::<coverage::Model>(Stmt::new(
                "SELECT * FROM coverages WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    /// Lock the coverage's row for the rest of the transaction and check it is the version the caller read
    /// (OFH-11). `expected` is the `updated_at` the caller saw; `None` (the caller sent `*`) skips the
    /// comparison but still takes the lock, so a second writer waits and then gets `412`.
    pub async fn lock_if_current<C: Conn>(
        conn: &C,
        id: Uuid,
        expected: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Coverage> {
        let m = conn
            .one_as::<coverage::Model>(Stmt::new(
                "SELECT * FROM coverages WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("coverage {id}")))?;
        let c = from_model(m)?;
        if !crate::api::rest::concurrency::is_current(c.updated_at, expected) {
            return Err(Error::precondition_failed(
                "the coverage has changed since it was read; read it again",
            ));
        }
        Ok(c)
    }

    /// Replace the row in place. Caller is expected to have produced a
    /// merged record (existing-row + patch) before calling. Returns the row as stored, so its `updated_at`
    /// (to the microsecond) is the version the next write must send.
    pub async fn update<C: Conn>(conn: &C, c: &Coverage) -> Result<Coverage> {
        let mut row = to_row(c);
        row.updated_at = chrono::Utc::now().fixed_offset();
        let stored = row.update_row(conn).await?;
        from_model(stored)
    }

    /// Flip just the status column. Used by the soft-cancel endpoint
    /// (`DELETE /api/coverages/:id` → status = Cancelled).
    pub async fn set_status<C: Conn>(
        conn: &C,
        id: Uuid,
        new_status: CoverageStatus,
    ) -> Result<Coverage> {
        let m = conn
            .one_as::<coverage::Model>(Stmt::new(
                "SELECT * FROM coverages WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("coverage {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set("status", new_status.as_str().to_string());
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        from_model(updated)
    }

    /// Attach a coverage to a billing account, or detach by passing
    /// `None`.
    pub async fn link_to_account<C: Conn>(
        conn: &C,
        id: Uuid,
        account_id: Option<Uuid>,
    ) -> Result<Coverage> {
        let m = conn
            .one_as::<coverage::Model>(Stmt::new(
                "SELECT * FROM coverages WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("coverage {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set("account_id", account_id);
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        from_model(updated)
    }

    /// All coverage rows for a patient, newest-first.
    pub async fn list_by_patient<C: Conn>(conn: &C, person_id: Uuid) -> Result<Vec<Coverage>> {
        let rows = conn
            .all_as::<coverage::Model>(Stmt::new(
                "SELECT * FROM coverages WHERE person_id = $1 ORDER BY created_at DESC",
                [person_id.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    /// All coverage rows attached to a billing account, newest-first.
    /// Useful when surfacing payer info on an invoice or statement.
    pub async fn list_by_account<C: Conn>(conn: &C, account_id: Uuid) -> Result<Vec<Coverage>> {
        let rows = conn
            .all_as::<coverage::Model>(Stmt::new(
                "SELECT * FROM coverages WHERE account_id = $1 ORDER BY created_at DESC",
                [account_id.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }
}

// --- conversions ---

fn status_from_str(s: &str) -> Result<CoverageStatus> {
    match s {
        "active" => Ok(CoverageStatus::Active),
        "cancelled" => Ok(CoverageStatus::Cancelled),
        "draft" => Ok(CoverageStatus::Draft),
        "entered_in_error" => Ok(CoverageStatus::EnteredInError),
        other => Err(Error::internal(format!("unknown coverage status: {other}"))),
    }
}

fn kind_from_str(s: &str) -> Result<CoverageKind> {
    match s {
        "insurance" => Ok(CoverageKind::Insurance),
        "self_pay" => Ok(CoverageKind::SelfPay),
        "other" => Ok(CoverageKind::Other),
        other => Err(Error::internal(format!("unknown coverage kind: {other}"))),
    }
}

fn to_row(c: &Coverage) -> coverage::Model {
    coverage::Model {
        id: c.id,
        person_id: c.person_id,
        account_id: c.account_id,
        status: c.status.as_str().to_string(),
        kind: c.kind.as_str().to_string(),
        subscriber_id: c.subscriber_id,
        payor_name: c.payor_name.clone(),
        payor_identifier: c.payor_identifier.clone(),
        policy_number: c.policy_number.clone(),
        group_number: c.group_number.clone(),
        relationship: c.relationship.clone(),
        start_date: c.start_date,
        end_date: c.end_date,
        created_at: c.created_at.fixed_offset(),
        updated_at: c.updated_at.fixed_offset(),
    }
}

fn from_model(m: coverage::Model) -> Result<Coverage> {
    Ok(Coverage {
        id: m.id,
        person_id: m.person_id,
        account_id: m.account_id,
        status: status_from_str(&m.status)?,
        kind: kind_from_str(&m.kind)?,
        subscriber_id: m.subscriber_id,
        payor_name: m.payor_name,
        payor_identifier: m.payor_identifier,
        policy_number: m.policy_number,
        group_number: m.group_number,
        relationship: m.relationship,
        start_date: m.start_date,
        end_date: m.end_date,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coverage_repository_is_zero_sized() {
        assert_eq!(std::mem::size_of::<CoverageRepository>(), 0);
    }

    #[test]
    fn test_status_string_round_trip_via_helpers() {
        for s in [
            CoverageStatus::Active,
            CoverageStatus::Cancelled,
            CoverageStatus::Draft,
            CoverageStatus::EnteredInError,
        ] {
            assert_eq!(status_from_str(s.as_str()).unwrap(), s);
        }
        assert!(status_from_str("not-a-real-status").is_err());
    }

    #[test]
    fn test_kind_string_round_trip_via_helpers() {
        for k in [
            CoverageKind::Insurance,
            CoverageKind::SelfPay,
            CoverageKind::Other,
        ] {
            assert_eq!(kind_from_str(k.as_str()).unwrap(), k);
        }
        assert!(kind_from_str("not-a-real-kind").is_err());
    }

    #[test]
    fn test_to_active_model_round_trip_via_explicit_model() {
        let mut c = Coverage::new(Uuid::new_v4(), "Aetna", "P-1");
        c.account_id = Some(Uuid::new_v4());
        c.kind = CoverageKind::SelfPay;
        let am = to_row(&c);
        let m = coverage::Model {
            id: am.id,
            person_id: am.person_id,
            account_id: am.account_id,
            status: am.status.clone(),
            kind: am.kind.clone(),
            subscriber_id: am.subscriber_id,
            payor_name: am.payor_name.clone(),
            payor_identifier: am.payor_identifier.clone(),
            policy_number: am.policy_number.clone(),
            group_number: am.group_number.clone(),
            relationship: am.relationship.clone(),
            start_date: am.start_date,
            end_date: am.end_date,
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, c.id);
        assert_eq!(back.account_id, c.account_id);
        assert_eq!(back.kind, CoverageKind::SelfPay);
        assert_eq!(back.payor_name, "Aetna");
        assert_eq!(back.policy_number, "P-1");
    }
}
