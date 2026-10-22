//! Group consultations (NEW-18): a slot can declare how many patients it takes. 1 is a one-to-one
//! appointment, as every slot was before; more is a group session (a virtual group clinic, an education
//! session). The capacity is declared before anyone books: it is not overbooking.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE slots ADD COLUMN IF NOT EXISTS capacity integer NOT NULL DEFAULT 1",
            "ALTER TABLE slots ADD CONSTRAINT slots_capacity_chk CHECK (capacity BETWEEN 1 AND 50)",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "ALTER TABLE slots DROP CONSTRAINT IF EXISTS slots_capacity_chk",
            "ALTER TABLE slots DROP COLUMN IF EXISTS capacity",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}
