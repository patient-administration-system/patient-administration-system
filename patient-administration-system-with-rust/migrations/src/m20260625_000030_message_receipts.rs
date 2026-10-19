//! NOT-4: what happened to a message after it left us.
//!
//! A carrier reports `delivered`, `bounced` (with a reason code) or `replied` for a letter, by the id we gave
//! it as its reference. Each report is stored once (`carrier` + `receipt_id`), so a carrier that retries its
//! webhook is harmless. `contact_problem_resolutions` records that a person looked at a bouncing contact
//! and dealt with it, so bounces before that stop counting (CAS-9).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS message_receipts ( \
               id uuid PRIMARY KEY, \
               letter_id uuid NOT NULL, \
               event text NOT NULL CHECK (event IN ('delivered','bounced','replied')), \
               carrier text NOT NULL CHECK (length(btrim(carrier)) > 0), \
               receipt_id text NOT NULL CHECK (length(btrim(receipt_id)) BETWEEN 1 AND 200), \
               detail text NULL CHECK (detail IS NULL OR length(detail) <= 200), \
               occurred_at timestamptz NOT NULL, \
               created_at timestamptz NOT NULL DEFAULT now(), \
               UNIQUE (carrier, receipt_id))",
            "CREATE INDEX IF NOT EXISTS idx_message_receipts_letter ON message_receipts (letter_id, occurred_at)",
            "CREATE INDEX IF NOT EXISTS idx_message_receipts_bounced ON message_receipts (occurred_at) WHERE event = 'bounced'",
            "CREATE TABLE IF NOT EXISTS contact_problem_resolutions ( \
               id uuid PRIMARY KEY, \
               person_id uuid NOT NULL, \
               resolved_by text NOT NULL CHECK (length(btrim(resolved_by)) > 0), \
               note text NULL CHECK (note IS NULL OR length(note) <= 200), \
               resolved_at timestamptz NOT NULL DEFAULT now())",
            "CREATE INDEX IF NOT EXISTS idx_contact_resolutions_person ON contact_problem_resolutions (person_id, resolved_at DESC)",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for t in ["contact_problem_resolutions", "message_receipts"] {
            db.execute_unprepared(&format!("DROP TABLE IF EXISTS {t}"))
                .await?;
        }
        Ok(())
    }
}
