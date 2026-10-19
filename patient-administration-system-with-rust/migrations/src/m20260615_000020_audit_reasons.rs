//! CAP-6: any change or contact can be explained.
//!
//! - `audit_log.reason_code` and `context`: a short queryable code for why (for example the barrier a
//!   patient gave, `no_show`, `rescheduled`) and the facts around it, beside who (`user_id`) and when (`at`).
//! - `generated_letters.trigger` and `trigger_ref`: what caused a message to be sent (a reminder step, a
//!   staff member generating it) and the record it refers to. Existing rows read `manual`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "ALTER TABLE audit_log \
               ADD COLUMN IF NOT EXISTS reason_code text NULL, \
               ADD COLUMN IF NOT EXISTS context jsonb NULL",
            "ALTER TABLE generated_letters \
               ADD COLUMN IF NOT EXISTS trigger text NOT NULL DEFAULT 'manual', \
               ADD COLUMN IF NOT EXISTS trigger_ref text NULL",
            "CREATE INDEX IF NOT EXISTS idx_generated_letters_person ON generated_letters (person_id, created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_audit_log_entity ON audit_log (entity_type, entity_id, at)",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
