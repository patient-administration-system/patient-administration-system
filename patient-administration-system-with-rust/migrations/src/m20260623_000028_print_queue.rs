//! NOT-7: the postal letter print queue.
//!
//! The queue is every `print` letter still `pending` that is not in a batch. Exporting makes a batch (at
//! most one batch per letter); marking the batch sent flips exactly its letters to `sent`. The postal
//! address is never stored here: it is read from person-service when the batch is exported.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS print_batches ( \
               id uuid PRIMARY KEY, \
               created_by text NOT NULL CHECK (length(btrim(created_by)) > 0), \
               created_at timestamptz NOT NULL DEFAULT now(), \
               sent_at timestamptz NULL, \
               sent_by text NULL, \
               CHECK ((sent_at IS NULL AND sent_by IS NULL) OR (sent_at IS NOT NULL AND length(btrim(sent_by)) > 0)))",
            "CREATE TABLE IF NOT EXISTS print_batch_letters ( \
               batch_id uuid NOT NULL REFERENCES print_batches (id), \
               letter_id uuid NOT NULL UNIQUE, \
               PRIMARY KEY (batch_id, letter_id))",
            "CREATE INDEX IF NOT EXISTS idx_generated_letters_print_pending ON generated_letters (created_at) \
               WHERE channel = 'print' AND status = 'pending'",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for t in ["print_batch_letters", "print_batches"] {
            db.execute_unprepared(&format!("DROP TABLE IF EXISTS {t}")).await?;
        }
        Ok(())
    }
}
