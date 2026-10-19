//! PILOT-7: per-channel opt-outs, and the inbound-text ledger learns START.
//!
//! An opt-out is held against a *subject*: `person:<uuid>` when we know who asked, and `address:<number
//! or email>` always (a STOP from a shared or unknown number must still stop sends to that number). A
//! send is blocked if either subject has an active opt-out for the channel. `resumed_at` is set when the
//! patient texts START; the row is kept so the history is not lost.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS message_optouts ( \
               id uuid PRIMARY KEY, \
               subject text NOT NULL CHECK (subject ~ '^(person|address):.+'), \
               channel text NOT NULL CHECK (channel IN ('sms','whatsapp','apple_messages','email','voice')), \
               opted_out_at timestamptz NOT NULL DEFAULT now(), \
               resumed_at timestamptz NULL, \
               source text NOT NULL CHECK (source IN ('reply_stop','staff','portal')), \
               recorded_by text NULL, \
               UNIQUE (subject, channel))",
            "ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_intent_check",
            "ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_intent_check \
               CHECK (intent IN ('confirm','cancel','reschedule','stop','start','help','choice','unknown'))",
            "ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_outcome_check",
            "ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_outcome_check \
               CHECK (outcome IN ('applied','rejected','asked','needs_staff','stop','start','help','unknown','unmatched','ambiguous_number','no_open_reminder'))",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS message_optouts")
            .await?;
        db.execute_unprepared(
            "DELETE FROM inbound_texts WHERE intent = 'start' OR outcome = 'start'",
        )
        .await?;
        for ddl in [
            "ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_intent_check",
            "ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_intent_check \
               CHECK (intent IN ('confirm','cancel','reschedule','stop','help','choice','unknown'))",
            "ALTER TABLE inbound_texts DROP CONSTRAINT IF EXISTS inbound_texts_outcome_check",
            "ALTER TABLE inbound_texts ADD CONSTRAINT inbound_texts_outcome_check \
               CHECK (outcome IN ('applied','rejected','asked','needs_staff','stop','help','unknown','unmatched','ambiguous_number','no_open_reminder'))",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }
}
