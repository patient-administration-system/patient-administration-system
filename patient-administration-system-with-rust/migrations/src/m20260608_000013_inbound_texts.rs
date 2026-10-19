//! PILOT-4: a ledger of every inbound text reply, whatever became of it. It is the idempotency key for
//! the carrier's message id, the memory of an "which appointment?" question awaiting its number, and
//! the staff list of replies nobody could act on (reschedule requests, unknown numbers, shared numbers).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS inbound_texts ( \
               id uuid PRIMARY KEY, \
               message_id text NOT NULL UNIQUE, \
               carrier text NULL, \
               from_e164 text NOT NULL CHECK (from_e164 ~ '^\\+[1-9][0-9]{7,14}$'), \
               body text NOT NULL CHECK (char_length(body) <= 500), \
               received_at timestamptz NOT NULL, \
               person_id uuid NULL, \
               intent text NOT NULL CHECK (intent IN ('confirm','cancel','reschedule','stop','help','choice','unknown')), \
               outcome text NOT NULL CHECK (outcome IN ('applied','rejected','asked','needs_staff','stop','help','unknown','unmatched','ambiguous_number','no_open_reminder')), \
               appointment_id uuid NULL, \
               candidates jsonb NULL, \
               pending_intent text NULL CHECK (pending_intent IS NULL OR pending_intent IN ('confirm','cancel','reschedule')), \
               reply_text text NULL, \
               handled_at timestamptz NULL, \
               created_at timestamptz NOT NULL DEFAULT now())",
            "CREATE INDEX IF NOT EXISTS idx_inbound_texts_from ON inbound_texts (from_e164, received_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_inbound_texts_staff ON inbound_texts (outcome, created_at) \
               WHERE handled_at IS NULL AND outcome IN ('needs_staff','unmatched','ambiguous_number','no_open_reminder','unknown','rejected')",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS inbound_texts")
            .await?;
        Ok(())
    }
}
