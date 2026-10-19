//! CAS-7: wording experiments.
//!
//! An experiment compares wordings of one message. Each variant is a complete template group (every required
//! language), and each appointment is assigned one variant deterministically, once, and the assignment is
//! recorded so the result can be read out afterwards. An experiment changes wording only: nothing about
//! who is offered what.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS message_experiments ( \
               id uuid PRIMARY KEY, \
               template_name text NOT NULL CHECK (length(btrim(template_name)) > 0), \
               started_by text NOT NULL CHECK (length(btrim(started_by)) > 0), \
               started_at timestamptz NOT NULL DEFAULT now(), \
               stopped_at timestamptz NULL, \
               stopped_by text NULL, \
               CHECK ((stopped_at IS NULL AND stopped_by IS NULL) OR (stopped_at IS NOT NULL AND length(btrim(stopped_by)) > 0)))",
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_message_experiments_live ON message_experiments (template_name) WHERE stopped_at IS NULL",
            "CREATE TABLE IF NOT EXISTS message_experiment_variants ( \
               experiment_id uuid NOT NULL REFERENCES message_experiments (id), \
               variant text NOT NULL CHECK (variant ~ '^[A-Z]$'), \
               template_name text NOT NULL, \
               PRIMARY KEY (experiment_id, variant))",
            "CREATE TABLE IF NOT EXISTS message_experiment_assignments ( \
               experiment_id uuid NOT NULL REFERENCES message_experiments (id), \
               appointment_id uuid NOT NULL, \
               variant text NOT NULL, \
               assigned_at timestamptz NOT NULL DEFAULT now(), \
               PRIMARY KEY (experiment_id, appointment_id))",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for t in [
            "message_experiment_assignments",
            "message_experiment_variants",
            "message_experiments",
        ] {
            db.execute_unprepared(&format!("DROP TABLE IF EXISTS {t}"))
                .await?;
        }
        Ok(())
    }
}
