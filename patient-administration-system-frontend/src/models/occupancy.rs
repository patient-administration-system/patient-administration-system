//! Read-side join: bed → active bed_assignment → encounter → person (name from person-service).
//!
//! Returns the *current* occupant (if any) of each bed in a given set.

use crate::models::patient::names_for;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};
use std::collections::HashMap;
use uuid::Uuid;

mod bed_assignment_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "bed_assignments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub encounter_id: Uuid,
        pub bed_id: Uuid,
        pub assigned_at: chrono::DateTime<chrono::FixedOffset>,
        pub released_at: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod encounter_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "encounters")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub person_id: Uuid,
        pub class: String,
        pub status: String,
        pub period_start: chrono::DateTime<chrono::FixedOffset>,
        pub period_end: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub worker_id: Option<Uuid>,
        pub department_id: Option<Uuid>,
        pub reason: Option<String>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
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
    db: &DatabaseConnection,
    bed_ids: &[Uuid],
) -> Result<HashMap<Uuid, Occupant>, DbErr> {
    if bed_ids.is_empty() {
        return Ok(HashMap::new());
    }

    // 1. Active bed_assignments for the given beds.
    let assignments = bed_assignment_entity::Entity::find()
        .filter(bed_assignment_entity::Column::BedId.is_in(bed_ids.to_vec()))
        .filter(bed_assignment_entity::Column::ReleasedAt.is_null())
        .all(db)
        .await?;
    if assignments.is_empty() {
        return Ok(HashMap::new());
    }

    // 2. Look up the encounters those assignments belong to.
    let encounter_ids: Vec<Uuid> = assignments.iter().map(|a| a.encounter_id).collect();
    let encounters = encounter_entity::Entity::find()
        .filter(encounter_entity::Column::Id.is_in(encounter_ids))
        .all(db)
        .await?;
    let patient_by_encounter: HashMap<Uuid, Uuid> =
        encounters.iter().map(|e| (e.id, e.person_id)).collect();

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
