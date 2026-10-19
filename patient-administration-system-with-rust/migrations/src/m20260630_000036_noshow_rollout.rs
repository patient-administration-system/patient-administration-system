//! CAP-12 evaluation design: shadow mode first, then a stepped rollout by clinic.
//!
//! Every service starts in shadow: the probability is computed and stored (so it can be checked against what
//! happened) but is not shown and creates no calls. A named person moves one clinic at a time to live, which
//! is also the comparison the evaluation reads: clinics not yet live are the contrast for those that are.
//! No patient is ever denied support by being in a comparison group: the standard reminders and the silent
//! patient ladder apply everywhere; the score only adds calls.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS noshow_rollout ( \
                   service_type text PRIMARY KEY, \
                   stage text NOT NULL CHECK (stage IN ('shadow','live')), \
                   changed_by text NOT NULL CHECK (length(btrim(changed_by)) > 0), \
                   note text NOT NULL, \
                   changed_at timestamptz NOT NULL DEFAULT now(), \
                   live_since timestamptz NULL)",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS noshow_rollout")
            .await?;
        Ok(())
    }
}
