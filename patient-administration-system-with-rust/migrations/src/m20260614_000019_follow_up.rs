//! CAP-7: a person follows up a missed appointment on services a clinician has listed.
//!
//! - `follow_up_policies`: the services whose missed appointments must always get a person's follow-up,
//!   with who approved the listing. A listing without an approver is refused.
//! - `follow_up_tasks`: one per missed appointment on such a service. It can be closed only by a person
//!   recording an outcome and their name; nothing closes it automatically.
//!
//! A miss triggers contact. It never discharges the patient, removes them from a list, or touches their
//! referral-to-treatment clock.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS follow_up_policies ( \
               service_type text PRIMARY KEY, \
               approved_by text NOT NULL CHECK (length(btrim(approved_by)) > 0), \
               approved_at timestamptz NOT NULL DEFAULT now())",
            "CREATE TABLE IF NOT EXISTS follow_up_tasks ( \
               id uuid PRIMARY KEY, \
               appointment_id uuid NOT NULL UNIQUE, \
               person_id uuid NOT NULL, \
               service_type text NOT NULL, \
               due_at timestamptz NOT NULL, \
               created_at timestamptz NOT NULL DEFAULT now(), \
               outcome text NULL CHECK (outcome IS NULL OR outcome IN \
                 ('reached_rebooked','reached_will_attend','reached_no_longer_needed','no_answer','wrong_number','declined_help','clinical_escalation')), \
               closed_by text NULL, \
               closed_at timestamptz NULL, \
               CHECK ((outcome IS NULL AND closed_by IS NULL AND closed_at IS NULL) \
                   OR (outcome IS NOT NULL AND closed_at IS NOT NULL AND length(btrim(closed_by)) > 0)))",
            "CREATE INDEX IF NOT EXISTS idx_follow_up_tasks_open ON follow_up_tasks (due_at) WHERE closed_at IS NULL",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for t in ["follow_up_tasks", "follow_up_policies"] {
            db.execute_unprepared(&format!("DROP TABLE IF EXISTS {t}"))
                .await?;
        }
        Ok(())
    }
}
