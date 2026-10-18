//! PILOT-2: widen the barrier codes. A patient who cannot come says why in their own terms; five more
//! reasons are now recognised (anxiety or fear, cost, no longer needed, could not cancel or did not
//! know how, and a communication or access need). The set is a CHECK so no route can store a value
//! the application does not understand.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const OLD: &str = "'cannot_travel','caring','work','feel_better','unwell','forgot','other'";
const NEW: &str = "'cannot_travel','caring','work','feel_better','unwell','forgot','other',\
                   'anxiety','cost','no_longer_needed','could_not_cancel','access_need'";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        swap(manager, NEW).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Rows already using a new code would violate the old constraint; fold them into `other`.
        let db = manager.get_connection();
        db.execute_unprepared(
            "UPDATE appointments SET barrier_code = 'other' WHERE barrier_code IN \
             ('anxiety','cost','no_longer_needed','could_not_cancel','access_need')",
        )
        .await?;
        swap(manager, OLD).await
    }
}

async fn swap(manager: &SchemaManager<'_>, codes: &str) -> Result<(), DbErr> {
    let db = manager.get_connection();
    db.execute_unprepared(
        "ALTER TABLE appointments DROP CONSTRAINT IF EXISTS appointments_barrier_code_chk",
    )
    .await?;
    db.execute_unprepared(&format!(
        "ALTER TABLE appointments ADD CONSTRAINT appointments_barrier_code_chk \
         CHECK (barrier_code IS NULL OR barrier_code IN ({codes}))"
    ))
    .await?;
    Ok(())
}
