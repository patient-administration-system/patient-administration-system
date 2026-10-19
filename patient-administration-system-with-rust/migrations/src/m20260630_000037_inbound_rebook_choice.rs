//! A "different time" text reply is answered with real alternatives (CAP-5), so a question the PAS has put to
//! the patient can now be "rebook": the stored candidates are the appointment followed by the offered slots.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_pending_intent_check")
            .await?;
        db.execute_unprepared(
            "ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_pending_intent_check \
             CHECK (pending_intent IS NULL OR pending_intent IN ('confirm','cancel','reschedule','rebook'))",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("UPDATE inbound_texts SET pending_intent = 'reschedule' WHERE pending_intent = 'rebook'")
            .await?;
        db.execute_unprepared("ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_pending_intent_check")
            .await?;
        db.execute_unprepared(
            "ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_pending_intent_check \
             CHECK (pending_intent IS NULL OR pending_intent IN ('confirm','cancel','reschedule'))",
        )
        .await?;
        Ok(())
    }
}
