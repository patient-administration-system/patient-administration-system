//! Read-side join: bed → active bed_assignment → encounter → person (name from person-service).
//!
//! Returns the *current* occupant (if any) of each bed in a given set.

use crate::models::patient::names_for;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct AssignmentRecord {
    encounter_id: Uuid,
    bed_id: Uuid,
}

/// Current occupant of one bed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Occupant {
    pub person_id: Uuid,
    pub family: String,
    pub given: String,
}

/// For each bed in `bed_ids`, return the current occupant (if the bed has
/// an active assignment). Beds with no occupant are simply absent from
/// the returned map.
pub async fn current_occupants_for_beds(
    db: &PgPool,
    bed_ids: &[Uuid],
) -> Result<HashMap<Uuid, Occupant>, sqlx::Error> {
    if bed_ids.is_empty() {
        return Ok(HashMap::new());
    }

    // 1. Active bed_assignments for the given beds.
    let assignments: Vec<AssignmentRecord> = sqlx::query_as(
        "SELECT encounter_id, bed_id FROM bed_assignments \
         WHERE bed_id = ANY($1) AND released_at IS NULL",
    )
    .bind(bed_ids)
    .fetch_all(db)
    .await?;
    if assignments.is_empty() {
        return Ok(HashMap::new());
    }

    // 2. Look up the encounters those assignments belong to.
    let encounter_ids: Vec<Uuid> = assignments.iter().map(|a| a.encounter_id).collect();
    let patient_by_encounter: HashMap<Uuid, Uuid> =
        sqlx::query_as("SELECT id, person_id FROM encounters WHERE id = ANY($1)")
            .bind(encounter_ids)
            .fetch_all(db)
            .await?
            .into_iter()
            .collect();

    // 3. Look up names in person-service (identity is not stored here).
    let person_ids: Vec<Uuid> = patient_by_encounter.values().copied().collect();
    if person_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let names = names_for(&person_ids).await;

    // 4. Stitch: bed_id → encounter → patient.
    let mut out: HashMap<Uuid, Occupant> = HashMap::with_capacity(assignments.len());
    for a in assignments {
        let Some(person_id) = patient_by_encounter.get(&a.encounter_id).copied() else {
            continue;
        };
        // An occupied bed stays occupied even when the name is unavailable.
        let (family, given) = names
            .get(&person_id)
            .cloned()
            .unwrap_or_else(|| ("(unknown)".into(), String::new()));
        out.insert(
            a.bed_id,
            Occupant {
                person_id,
                family,
                given,
            },
        );
    }
    Ok(out)
}
