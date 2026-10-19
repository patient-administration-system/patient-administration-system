//! CAP-13: the reminder ladder's memory.
//!
//! - `reminder_steps`: one row per appointment per ladder step, so a step runs once and every step can
//!   be reported by channel, language and outcome (and, with CAP-11, split by deprivation).
//! - `staff_call_tasks`: appointments a person should phone (a silent patient, a reschedule request).
//! - `reply_tokens`: single-use links in messages. Only a hash of the token is stored, so a database
//!   leak does not hand out working links; the token itself carries no personal data.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS reminder_steps ( \
               id uuid PRIMARY KEY, \
               appointment_id uuid NOT NULL, \
               step text NOT NULL CHECK (step IN ('main','second','call','final')), \
               planned_for timestamptz NOT NULL, \
               channel text NULL, \
               locale text NULL, \
               outcome text NOT NULL CHECK (outcome IN ('sent','failed','skipped_replied','skipped_late','skipped_no_channel','skipped_opted_out','capped','call_task')), \
               letter_id uuid NULL, \
               created_at timestamptz NOT NULL DEFAULT now(), \
               UNIQUE (appointment_id, step))",
            "CREATE INDEX IF NOT EXISTS idx_reminder_steps_outcome ON reminder_steps (step, outcome, created_at)",
            "CREATE TABLE IF NOT EXISTS staff_call_tasks ( \
               id uuid PRIMARY KEY, \
               appointment_id uuid NOT NULL, \
               person_id uuid NOT NULL, \
               reason text NOT NULL CHECK (reason IN ('silent','reschedule_request','urgent_pathway')), \
               due_at timestamptz NOT NULL, \
               created_at timestamptz NOT NULL DEFAULT now(), \
               handled_at timestamptz NULL, \
               handled_by text NULL, \
               call_outcome text NULL CHECK (call_outcome IS NULL OR call_outcome IN ('reached','no_answer','declined_help','wrong_number')), \
               UNIQUE (appointment_id, reason))",
            "CREATE INDEX IF NOT EXISTS idx_staff_call_tasks_open ON staff_call_tasks (due_at) WHERE handled_at IS NULL",
            "CREATE TABLE IF NOT EXISTS reply_tokens ( \
               token_hash text PRIMARY KEY CHECK (token_hash ~ '^[0-9a-f]{64}$'), \
               appointment_id uuid NOT NULL, \
               person_id uuid NOT NULL, \
               expires_at timestamptz NOT NULL, \
               consumed_at timestamptz NULL, \
               consumed_action text NULL CHECK (consumed_action IS NULL OR consumed_action IN ('confirm','cancel','reschedule')), \
               created_at timestamptz NOT NULL DEFAULT now())",
            "CREATE INDEX IF NOT EXISTS idx_reply_tokens_appointment ON reply_tokens (appointment_id)",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for t in ["reply_tokens", "staff_call_tasks", "reminder_steps"] {
            db.execute_unprepared(&format!("DROP TABLE IF EXISTS {t}"))
                .await?;
        }
        Ok(())
    }
}
