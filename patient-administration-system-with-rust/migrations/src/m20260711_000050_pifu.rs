//! Patient-initiated follow-up (NEW-3). A clinician opens an episode for a patient with a window and the
//! conditions written for the patient; inside the window a request from the patient is a booking task,
//! after it a person decides. At the end of the window a person reviews the episode (discharge or extend):
//! nothing discharges a patient by itself.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "CREATE TABLE IF NOT EXISTS pifu_episodes ( \
               id uuid PRIMARY KEY, \
               person_id uuid NOT NULL, \
               service_type text NOT NULL CHECK (length(btrim(service_type)) BETWEEN 1 AND 100), \
               conditions text NOT NULL CHECK (length(btrim(conditions)) BETWEEN 10 AND 1000), \
               opened_by text NOT NULL CHECK (length(btrim(opened_by)) > 0), \
               opened_at timestamptz NOT NULL, \
               expires_at timestamptz NOT NULL CHECK (expires_at > opened_at), \
               urgent_override_by text CHECK (urgent_override_by IS NULL OR length(btrim(urgent_override_by)) > 0), \
               extended_count integer NOT NULL DEFAULT 0, \
               status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','closed')), \
               closed_by text, \
               closed_at timestamptz, \
               CHECK ((status = 'closed') = (closed_at IS NOT NULL AND length(btrim(closed_by)) > 0)))",
            // One live episode per patient and service.
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_pifu_episodes_live ON pifu_episodes (person_id, service_type) \
               WHERE status = 'active'",
            "CREATE TABLE IF NOT EXISTS pifu_requests ( \
               id uuid PRIMARY KEY, \
               episode_id uuid NOT NULL REFERENCES pifu_episodes (id), \
               person_id uuid NOT NULL, \
               requested_at timestamptz NOT NULL, \
               channel text NOT NULL CHECK (channel IN ('staff','phone','text','reply_link','portal')), \
               route text NOT NULL CHECK (route IN ('booking','person')), \
               handled_by text, \
               handled_at timestamptz, \
               appointment_id uuid, \
               CHECK ((handled_at IS NULL) = (handled_by IS NULL)))",
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_pifu_requests_open ON pifu_requests (episode_id) \
               WHERE handled_at IS NULL",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for t in ["pifu_requests", "pifu_episodes"] {
            db.execute_unprepared(&format!("DROP TABLE IF EXISTS {t}"))
                .await?;
        }
        Ok(())
    }
}
