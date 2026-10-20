//! Tolerances per service or ward, and the alerts raised when a measure turns red.
//!
//! A tolerance row now belongs to a scope: the whole service (`all`, the key is empty), one `service` (by its
//! service type) or one `ward` (by its code). The alert table keeps one open alert per measure and scope, so a
//! measure that stays red is one alert, not one per check.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "ALTER TABLE tolerances \
               ADD COLUMN IF NOT EXISTS scope_kind text NOT NULL DEFAULT 'all' \
                 CHECK (scope_kind IN ('all','service','ward')), \
               ADD COLUMN IF NOT EXISTS scope_key text NOT NULL DEFAULT ''",
        )
        .await?;
        db.execute_unprepared(
            "DO $$ BEGIN \
               IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'tolerances_scope_matches_kind') THEN \
                 ALTER TABLE tolerances ADD CONSTRAINT tolerances_scope_matches_kind \
                   CHECK ((scope_kind = 'all') = (scope_key = '')); \
               END IF; \
             END $$",
        )
        .await?;
        db.execute_unprepared("ALTER TABLE tolerances DROP CONSTRAINT IF EXISTS tolerances_pkey")
            .await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_tolerances_scope ON tolerances (metric, scope_kind, scope_key)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS tolerance_alerts ( \
               id uuid PRIMARY KEY, \
               metric text NOT NULL, \
               scope_kind text NOT NULL CHECK (scope_kind IN ('all','service','ward')), \
               scope_key text NOT NULL DEFAULT '', \
               value double precision NOT NULL, \
               target double precision NOT NULL, \
               tolerance double precision NOT NULL, \
               escalate_beyond double precision NOT NULL, \
               raised_at timestamptz NOT NULL DEFAULT now(), \
               last_seen_at timestamptz NOT NULL DEFAULT now(), \
               cleared_at timestamptz NULL, \
               acknowledged_by text NULL CHECK (acknowledged_by IS NULL OR length(btrim(acknowledged_by)) > 0), \
               acknowledged_at timestamptz NULL, \
               note text NOT NULL DEFAULT '')",
        )
        .await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_tolerance_alerts_open \
               ON tolerance_alerts (metric, scope_kind, scope_key) WHERE cleared_at IS NULL",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS tolerance_alerts")
            .await?;
        db.execute_unprepared("DELETE FROM tolerances WHERE scope_kind <> 'all'")
            .await?;
        db.execute_unprepared("DROP INDEX IF EXISTS ux_tolerances_scope")
            .await?;
        db.execute_unprepared(
            "ALTER TABLE tolerances DROP CONSTRAINT IF EXISTS tolerances_scope_matches_kind, \
               DROP COLUMN IF EXISTS scope_kind, DROP COLUMN IF EXISTS scope_key",
        )
        .await?;
        db.execute_unprepared("ALTER TABLE tolerances ADD PRIMARY KEY (metric)")
            .await?;
        Ok(())
    }
}
