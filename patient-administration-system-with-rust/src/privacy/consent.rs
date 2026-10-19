//! Requiring a patient's consent before a use that needs it (DTAC-7).
//!
//! Consent is not the basis for the PAS's core work (direct care and the messages that go with it); it is
//! needed for the uses that go beyond that: marketing, research, sharing with third parties. A handler for
//! such a use calls [`require_consent`] first and answers 403 without it. Today the only such use is a
//! marketing message ([`crate::communication::CommunicationService`] refuses one without a
//! `marketing` consent). Any future research or sharing endpoint MUST call this before it reads or sends
//! anything about an identifiable patient; `governance/messaging-rules.md` says the same.

use chrono::Utc;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

use crate::db::repositories::consent::ConsentRepository;
use crate::models::consent::ConsentType;
use crate::{Error, Result};

/// Is there an active, unexpired, unrevoked consent of this type for the patient?
pub async fn has_consent<C: ConnectionTrait>(
    conn: &C,
    person_id: Uuid,
    kind: ConsentType,
) -> Result<bool> {
    let today = Utc::now().date_naive();
    Ok(ConsentRepository::list_for_patient(conn, person_id)
        .await?
        .iter()
        .any(|c| c.consent_type == kind && c.revoked_date.is_none() && c.is_active(today)))
}

/// Refuse (403) unless the patient has an active consent of this type.
pub async fn require_consent<C: ConnectionTrait>(
    conn: &C,
    person_id: Uuid,
    kind: ConsentType,
) -> Result<()> {
    if has_consent(conn, person_id, kind).await? {
        Ok(())
    } else {
        Err(Error::forbidden(format!(
            "the patient has not given an active {kind:?} consent"
        )))
    }
}
