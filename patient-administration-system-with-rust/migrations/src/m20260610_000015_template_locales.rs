//! PILOT-5 / CAP-9: templates carry a language. Templates with the same name are the language variants
//! of one message; only one active variant per name and language. Generated letters remember which
//! variant they used.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "ALTER TABLE letter_templates ADD COLUMN IF NOT EXISTS locale text NOT NULL DEFAULT 'en-001'",
            "ALTER TABLE letter_templates ADD CONSTRAINT letter_templates_locale_chk \
               CHECK (locale ~ '^[a-z]{2,3}(-[A-Za-z0-9]{1,8})*$' AND char_length(locale) <= 16)",
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_letter_templates_active_name_locale \
               ON letter_templates (name, locale) WHERE active",
            "ALTER TABLE generated_letters ADD COLUMN IF NOT EXISTS locale text NULL",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "DROP INDEX IF EXISTS uq_letter_templates_active_name_locale",
            "ALTER TABLE letter_templates DROP CONSTRAINT IF EXISTS letter_templates_locale_chk",
            "ALTER TABLE letter_templates DROP COLUMN IF EXISTS locale",
            "ALTER TABLE generated_letters DROP COLUMN IF EXISTS locale",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }
}
