//! Waiting-list validation campaigns (NEW-2): a cohort of long-waiting pathways is asked whether the care is
//! still needed. A patient who does not answer gets a letter; only then can a named person decide to remove
//! them. A removal is recorded as a validation removal (so it can be reported apart from treatment) and can be
//! reinstated, with the clock back where it was. Nothing here removes anyone by itself.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "CREATE TABLE validation_campaigns ( \
               id uuid PRIMARY KEY, \
               name text NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 200), \
               service_type text NULL, \
               min_weeks integer NOT NULL CHECK (min_weeks >= 1), \
               started_by text NOT NULL CHECK (length(btrim(started_by)) > 0), \
               reply_by date NOT NULL, \
               final_wait_days integer NOT NULL CHECK (final_wait_days BETWEEN 1 AND 90), \
               status text NOT NULL DEFAULT 'open' CHECK (status IN ('open','closed')), \
               created_at timestamptz NOT NULL DEFAULT now(), \
               closed_at timestamptz NULL, \
               closed_by text NULL, \
               CHECK ((status = 'closed') = (closed_at IS NOT NULL AND closed_by IS NOT NULL)))",
            "CREATE TABLE validation_items ( \
               id uuid PRIMARY KEY, \
               campaign_id uuid NOT NULL REFERENCES validation_campaigns(id), \
               pathway_id uuid NOT NULL REFERENCES rtt_pathways(id), \
               person_id uuid NOT NULL, \
               target_service text NOT NULL, \
               state text NOT NULL DEFAULT 'selected' CHECK (state IN \
                 ('selected','asked','answered','letter_sent','kept','removed','reinstated')), \
               check_id uuid NULL REFERENCES still_needed_checks(id), \
               asked_by text NULL, asked_channel text NULL, asked_at timestamptz NULL, \
               answer text NULL CHECK (answer IS NULL OR answer IN ('still_needed','no_longer_needed','unsure')), \
               answered_at timestamptz NULL, \
               letter_sent_by text NULL, letter_sent_at timestamptz NULL, final_by date NULL, \
               decided_by text NULL, decided_at timestamptz NULL, decision_note text NULL, \
               removed_at timestamptz NULL, rtt_stop_event_at timestamptz NULL, \
               removed_waitlist_ids uuid[] NOT NULL DEFAULT '{}', \
               reinstated_by text NULL, reinstated_at timestamptz NULL, \
               UNIQUE (campaign_id, pathway_id), \
               CHECK (state NOT IN ('kept','removed','reinstated') OR (length(btrim(decided_by)) > 0 AND decided_at IS NOT NULL)), \
               CHECK (state <> 'removed' OR removed_at IS NOT NULL))",
            // A pathway is in at most one live campaign item at a time.
            "CREATE UNIQUE INDEX ux_validation_items_live ON validation_items (pathway_id) \
               WHERE state IN ('selected','asked','answered','letter_sent')",
            "CREATE INDEX idx_validation_items_campaign ON validation_items (campaign_id, state)",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    /// Dropping the tables drops the campaigns' records; a removal made through one stays removed (the
    /// waiting-list entry and the stopped clock are not undone).
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS validation_items")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS validation_campaigns")
            .await?;
        Ok(())
    }
}
