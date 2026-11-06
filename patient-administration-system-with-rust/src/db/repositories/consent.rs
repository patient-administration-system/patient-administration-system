//! consent repository

use crate::db::sql::{Conn, Stmt, Table};
use uuid::Uuid;

use crate::db::entities::consent;
use crate::models::consent::{Consent, ConsentStatus, ConsentType};
use crate::{Error, Result};

pub struct ConsentRepository;

impl ConsentRepository {
    pub async fn create<C: Conn>(conn: &C, c: &Consent) -> Result<Consent> {
        let am = to_row(c);
        am.insert(conn).await?;
        Ok(c.clone())
    }

    pub async fn find_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Consent>> {
        let m = conn
            .one_as::<consent::Model>(Stmt::new(
                "SELECT * FROM consents WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(from_model).transpose()
    }

    pub async fn list_for_patient<C: Conn>(conn: &C, person_id: Uuid) -> Result<Vec<Consent>> {
        let rows = conn
            .all_as::<consent::Model>(Stmt::new(
                "SELECT * FROM consents WHERE person_id = $1",
                [person_id.into()],
            ))
            .await?;
        rows.into_iter().map(from_model).collect()
    }

    /// Mark a consent as Revoked. Sets `status = Revoked` and stamps
    /// `revoked_date` to today.
    /// Withdraw a consent. It needs no `If-Match` on purpose (OFH-11): a withdrawal is one-way, so a stale
    /// view cannot make it wrong, and refusing one would delay a patient's right to withdraw. The row is locked,
    /// and a consent already revoked is returned as it is, so a repeat cannot move the date it was withdrawn.
    pub async fn revoke<C: Conn>(conn: &C, id: Uuid) -> Result<Consent> {
        let m = conn
            .one_as::<consent::Model>(Stmt::new(
                "SELECT * FROM consents WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("consent {id}")))?;
        if m.status == consent_status_to_str(ConsentStatus::Revoked) {
            return from_model(m);
        }
        let today = chrono::Utc::now().date_naive();
        let mut am = m.patch();
        am.set(
            "status",
            consent_status_to_str(ConsentStatus::Revoked).to_string(),
        );
        am.set("revoked_date", Some(today));
        am.set("updated_at", chrono::Utc::now().fixed_offset());
        let updated = am.update(conn).await?;
        from_model(updated)
    }
}

// --- conversion helpers ---

pub(crate) fn consent_type_to_str(t: ConsentType) -> &'static str {
    match t {
        ConsentType::DataProcessing => "data_processing",
        ConsentType::DataSharing => "data_sharing",
        ConsentType::Marketing => "marketing",
        ConsentType::Research => "research",
        ConsentType::EmergencyAccess => "emergency_access",
    }
}

pub(crate) fn consent_type_from_str(s: &str) -> Result<ConsentType> {
    match s {
        "data_processing" => Ok(ConsentType::DataProcessing),
        "data_sharing" => Ok(ConsentType::DataSharing),
        "marketing" => Ok(ConsentType::Marketing),
        "research" => Ok(ConsentType::Research),
        "emergency_access" => Ok(ConsentType::EmergencyAccess),
        other => Err(Error::internal(format!("unknown consent type: {other}"))),
    }
}

pub(crate) fn consent_status_to_str(s: ConsentStatus) -> &'static str {
    match s {
        ConsentStatus::Active => "active",
        ConsentStatus::Revoked => "revoked",
        ConsentStatus::Expired => "expired",
    }
}

pub(crate) fn consent_status_from_str(s: &str) -> Result<ConsentStatus> {
    match s {
        "active" => Ok(ConsentStatus::Active),
        "revoked" => Ok(ConsentStatus::Revoked),
        "expired" => Ok(ConsentStatus::Expired),
        other => Err(Error::internal(format!("unknown consent status: {other}"))),
    }
}

fn to_row(c: &Consent) -> consent::Model {
    consent::Model {
        id: c.id,
        person_id: c.person_id,
        consent_type: consent_type_to_str(c.consent_type).to_string(),
        status: consent_status_to_str(c.status).to_string(),
        granted_date: c.granted_date,
        expiry_date: c.expiry_date,
        revoked_date: c.revoked_date,
        purpose: c.purpose.clone(),
        method: c.method.clone(),
        created_at: c.created_at.fixed_offset(),
        updated_at: c.updated_at.fixed_offset(),
    }
}

fn from_model(m: consent::Model) -> Result<Consent> {
    Ok(Consent {
        id: m.id,
        person_id: m.person_id,
        consent_type: consent_type_from_str(&m.consent_type)?,
        status: consent_status_from_str(&m.status)?,
        granted_date: m.granted_date,
        expiry_date: m.expiry_date,
        revoked_date: m.revoked_date,
        purpose: m.purpose,
        method: m.method,
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_consent_roundtrip_via_row() {
        let granted = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let c = Consent::new(Uuid::new_v4(), ConsentType::DataProcessing, granted);
        let am = to_row(&c);
        let m = consent::Model {
            id: am.id,
            person_id: am.person_id,
            consent_type: am.consent_type.clone(),
            status: am.status.clone(),
            granted_date: am.granted_date,
            expiry_date: am.expiry_date,
            revoked_date: am.revoked_date,
            purpose: am.purpose.clone(),
            method: am.method.clone(),
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = from_model(m).expect("from_model");
        assert_eq!(back.id, c.id);
        assert_eq!(back.consent_type, ConsentType::DataProcessing);
        assert_eq!(back.status, ConsentStatus::Active);
        assert_eq!(back.granted_date, granted);
    }
}
