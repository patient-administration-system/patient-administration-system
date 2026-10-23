//! Reasonable adjustments as a coded record (NEW-6).
//!
//! A named person records an adjustment a patient needs (large print, step-free access, a quiet room, a
//! longer appointment) from a fixed catalogue, with an optional short note and a review date. Staff see the
//! live adjustments when they book, and the same record is a FHIR `Flag`, shaped like the NHS England
//! Reasonable Adjustment Digital Flag (DAPB4019). The codes are the PAS's own under [`CODE_SYSTEM`]: the
//! national flag's SNOMED CT codes are not published in a form this repository could check, so the mapping
//! is left to the code system rather than guessed. An adjustment informs staff only; it never changes what
//! is offered (NFR-11).

use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

/// The FHIR code system the adjustment codes belong to.
pub const CODE_SYSTEM: &str =
    "https://patient-administration-system.example/fhir/CodeSystem/reasonable-adjustment";
/// The FHIR code system for the categories.
pub const CATEGORY_SYSTEM: &str =
    "https://patient-administration-system.example/fhir/CodeSystem/reasonable-adjustment-category";

/// Every adjustment code: `(category, code, display)`. Adding one here is the whole change: the database
/// stores the code as text and the list is checked on the way in.
pub const CATALOGUE: [(&str, &str, &str); 20] = [
    ("communication", "easy_read", "Easy read information"),
    ("communication", "large_print", "Large print"),
    ("communication", "braille", "Braille"),
    ("communication", "audio", "Audio information"),
    ("communication", "hearing_loop", "Hearing loop"),
    // A preference only (plan section 12): the PAS holds no pain score and no pain-scale artwork.
    (
        "communication",
        "pictorial_pain_scale",
        "Prefers a pictorial or simplified pain scale",
    ),
    (
        "communication",
        "text_not_calls",
        "Contact by text, not phone calls",
    ),
    ("physical_access", "step_free", "Step-free access"),
    (
        "physical_access",
        "wheelchair_space",
        "Space for a wheelchair",
    ),
    ("physical_access", "hoist", "Hoist or transfer equipment"),
    (
        "physical_access",
        "accessible_toilet",
        "Accessible toilet close by",
    ),
    (
        "physical_access",
        "assistance_dog",
        "Assistance dog accompanies the patient",
    ),
    ("sensory_environment", "quiet_space", "Quiet space to wait"),
    ("sensory_environment", "low_lighting", "Low lighting"),
    (
        "sensory_environment",
        "no_waiting_room",
        "Avoid the waiting room",
    ),
    (
        "understanding",
        "extra_time",
        "Extra time to take things in",
    ),
    (
        "understanding",
        "carer_present",
        "A carer or supporter present",
    ),
    (
        "appointment_handling",
        "longer_appointment",
        "A longer appointment",
    ),
    (
        "appointment_handling",
        "first_or_last_slot",
        "First or last appointment of a session",
    ),
    (
        "appointment_handling",
        "same_clinician",
        "The same clinician each time",
    ),
];

/// The category and display text of a code, or `None` when it is not in the catalogue.
pub fn lookup(code: &str) -> Option<(&'static str, &'static str)> {
    CATALOGUE
        .iter()
        .find(|(_, c, _)| *c == code)
        .map(|(cat, _, display)| (*cat, *display))
}

#[derive(Debug, Clone, Serialize)]
pub struct Adjustment {
    pub id: Uuid,
    pub person_id: Uuid,
    pub category: String,
    pub code: String,
    pub display: String,
    pub note: Option<String>,
    pub review_by: Option<NaiveDate>,
    pub recorded_by: String,
    pub recorded_at: DateTime<Utc>,
    pub ended_by: Option<String>,
    pub ended_at: Option<DateTime<Utc>>,
    /// The same adjustment as a FHIR `Flag`.
    pub fhir: Value,
}

/// An adjustment as a FHIR R5 `Flag`: active until ended, the code and category as codings, the note as the
/// code's text, and the period from when it was recorded.
pub fn to_flag(a: &Adjustment) -> Value {
    let mut period = json!({ "start": a.recorded_at.to_rfc3339() });
    if let Some(end) = a.ended_at {
        period["end"] = json!(end.to_rfc3339());
    }
    let mut code =
        json!({ "coding": [{ "system": CODE_SYSTEM, "code": a.code, "display": a.display }] });
    if let Some(note) = &a.note {
        code["text"] = json!(note);
    }
    json!({
        "resourceType": "Flag",
        "id": a.id,
        "status": if a.ended_at.is_some() { "inactive" } else { "active" },
        "category": [{ "coding": [{ "system": CATEGORY_SYSTEM, "code": a.category }] }],
        "code": code,
        "subject": { "reference": format!("Patient/{}", a.person_id) },
        "period": period,
    })
}

const COLS: &str =
    "id, person_id, category, code, note, review_by, recorded_by, recorded_at, ended_by, ended_at";

fn from_row(r: &sea_orm::QueryResult) -> Result<Adjustment> {
    let code: String = r.try_get("", "code").map_err(Error::Database)?;
    let display = lookup(&code).map_or_else(|| code.clone(), |(_, d)| d.to_string());
    let mut a = Adjustment {
        id: r.try_get("", "id").map_err(Error::Database)?,
        person_id: r.try_get("", "person_id").map_err(Error::Database)?,
        category: r.try_get("", "category").map_err(Error::Database)?,
        code,
        display,
        note: r.try_get("", "note").map_err(Error::Database)?,
        review_by: r.try_get("", "review_by").map_err(Error::Database)?,
        recorded_by: r.try_get("", "recorded_by").map_err(Error::Database)?,
        recorded_at: r.try_get("", "recorded_at").map_err(Error::Database)?,
        ended_by: r.try_get("", "ended_by").map_err(Error::Database)?,
        ended_at: r.try_get("", "ended_at").map_err(Error::Database)?,
        fhir: Value::Null,
    };
    a.fhir = to_flag(&a);
    Ok(a)
}

/// What a caller supplies to record an adjustment.
#[derive(Debug, Clone, Copy)]
pub struct NewAdjustment<'a> {
    pub code: &'a str,
    pub note: Option<&'a str>,
    pub review_by: Option<NaiveDate>,
    pub recorded_by: &'a str,
}

pub struct AdjustmentService {
    db: DatabaseConnection,
}

impl AdjustmentService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Record an adjustment for a patient. A named person; audited. The same code twice while the first is
    /// live is a conflict.
    pub async fn record(
        &self,
        person_id: Uuid,
        new: &NewAdjustment<'_>,
        at: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<Adjustment> {
        let NewAdjustment {
            code,
            note,
            review_by,
            recorded_by,
        } = *new;
        let by = named_person(
            recorded_by,
            "recorded_by: the person recording the adjustment is required",
        )?;
        let code = code.trim();
        let Some((category, _)) = lookup(code) else {
            return Err(Error::validation(format!(
                "adjustment code {code:?} is not in the catalogue (GET /api/reasonable-adjustments/codes)"
            )));
        };
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        if let Some(n) = note
            && (n.chars().count() > 200 || n.chars().any(char::is_control))
        {
            return Err(Error::validation(
                "note must be at most 200 characters and contain no control characters",
            ));
        }
        if let Some(d) = review_by
            && d < at.date_naive()
        {
            return Err(Error::validation("review_by may not be in the past"));
        }
        let id = Uuid::new_v4();
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let row = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!(
                    "INSERT INTO reasonable_adjustments (id, person_id, category, code, note, review_by, recorded_by, recorded_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                     ON CONFLICT (person_id, code) WHERE ended_at IS NULL DO NOTHING RETURNING {COLS}"
                ),
                [
                    id.into(),
                    person_id.into(),
                    category.into(),
                    code.into(),
                    note.into(),
                    review_by.into(),
                    by.into(),
                    at.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        let Some(row) = row else {
            return Err(Error::conflict(format!(
                "the patient already has a live {code} adjustment"
            )));
        };
        let adjustment = from_row(&row)?;
        AuditLogRepository::log(
            &txn,
            "reasonable_adjustment",
            id,
            "record",
            None,
            Some(json!({ "person_id": person_id, "code": code, "recorded_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(adjustment)
    }

    /// End a live adjustment (the need has gone, or was recorded in error). A named person; audited.
    pub async fn end(
        &self,
        id: Uuid,
        ended_by: &str,
        at: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            ended_by,
            "ended_by: the person ending the adjustment is required",
        )?;
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let done = txn
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE reasonable_adjustments SET ended_by = $2, ended_at = $3 WHERE id = $1 AND ended_at IS NULL",
                [id.into(), by.into(), at.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if done.rows_affected() != 1 {
            return Err(Error::not_found(format!("live adjustment {id}")));
        }
        AuditLogRepository::log(
            &txn,
            "reasonable_adjustment",
            id,
            "end",
            None,
            Some(json!({ "ended_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// The live adjustments of the patients of these appointments, for a schedule view: `(appointment_id,
    /// code)` pairs, at most 500 appointments. Read-only.
    pub async fn live_for_appointments(&self, ids: &[Uuid]) -> Result<Vec<(Uuid, String)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = ids.iter().take(500).copied().collect();
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT a.id AS appointment_id, r.code FROM appointments a \
                 JOIN reasonable_adjustments r ON r.person_id = a.person_id AND r.ended_at IS NULL \
                 WHERE a.id = ANY($1) ORDER BY a.id, r.category, r.code",
                [ids.into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok((
                    r.try_get("", "appointment_id").map_err(Error::Database)?,
                    r.try_get("", "code").map_err(Error::Database)?,
                ))
            })
            .collect()
    }

    /// A patient's adjustments, live ones first then ended ones when `include_ended`, newest first.
    pub async fn for_patient(
        &self,
        person_id: Uuid,
        include_ended: bool,
        fetch: i64,
    ) -> Result<Vec<Adjustment>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!(
                    "SELECT {COLS} FROM reasonable_adjustments \
                     WHERE person_id = $1 AND ($2 OR ended_at IS NULL) \
                     ORDER BY (ended_at IS NOT NULL), recorded_at DESC, id LIMIT $3"
                ),
                [
                    person_id.into(),
                    include_ended.into(),
                    crate::paging::fetch(fetch).into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter().map(from_row).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pictorial_pain_scale_preference_is_a_communication_code() {
        assert!(CATALOGUE.contains(&(
            "communication",
            "pictorial_pain_scale",
            "Prefers a pictorial or simplified pain scale"
        )));
    }

    #[test]
    fn the_catalogue_has_unique_codes_in_known_categories() {
        let mut seen = std::collections::HashSet::new();
        for (cat, code, _) in CATALOGUE {
            assert!(seen.insert(code), "{code} is listed twice");
            assert!(
                [
                    "communication",
                    "physical_access",
                    "sensory_environment",
                    "understanding",
                    "appointment_handling"
                ]
                .contains(&cat),
                "{cat} is not a category the table accepts"
            );
        }
    }

    #[test]
    fn a_flag_is_active_until_ended_and_carries_the_code_and_subject() {
        let now = Utc::now();
        let mut a = Adjustment {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            category: "communication".into(),
            code: "large_print".into(),
            display: "Large print".into(),
            note: Some("N24 font".into()),
            review_by: None,
            recorded_by: "Receptionist".into(),
            recorded_at: now,
            ended_by: None,
            ended_at: None,
            fhir: Value::Null,
        };
        let f = to_flag(&a);
        assert_eq!(f["resourceType"], "Flag");
        assert_eq!(f["status"], "active");
        assert_eq!(f["code"]["coding"][0]["code"], "large_print");
        assert_eq!(f["code"]["text"], "N24 font");
        assert_eq!(
            f["subject"]["reference"],
            format!("Patient/{}", a.person_id)
        );
        a.ended_at = Some(now);
        assert_eq!(to_flag(&a)["status"], "inactive");
    }
}
