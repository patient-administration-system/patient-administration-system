//! Proposed and held appointments for the IHE ITI Scheduling operations (NEW-10). `$find` records each slot
//! it proposes for a patient; `$hold` keeps one for a few minutes so nobody else can book it; `$book` turns
//! it into a real appointment through the same code as `POST /api/slots/{id}/book`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "CREATE TABLE IF NOT EXISTS slot_proposals ( \
               id uuid PRIMARY KEY, \
               slot_id uuid NOT NULL REFERENCES slots (id), \
               person_id uuid NOT NULL, \
               status text NOT NULL DEFAULT 'proposed' CHECK (status IN ('proposed','held','booked')), \
               expires_at timestamptz NOT NULL, \
               held_at timestamptz, \
               appointment_id uuid, \
               created_at timestamptz NOT NULL DEFAULT now(), \
               CHECK ((status = 'booked') = (appointment_id IS NOT NULL)), \
               CHECK (status <> 'held' OR held_at IS NOT NULL))",
            "CREATE INDEX IF NOT EXISTS idx_slot_proposals_slot ON slot_proposals (slot_id, status, expires_at)",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS slot_proposals")
            .await?;
        Ok(())
    }
}
