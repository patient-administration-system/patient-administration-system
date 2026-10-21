//! CAP-12: a no-show probability shown to staff, support only, behind a human gate.
//!
//! - `noshow_assessments`: one row per appointment, with the probability and the factors that made it, so
//!   staff see why. The band (low, medium, high) is *not* stored: it is derived from the probability and the
//!   configured thresholds when read, so changing a threshold changes the bands without recomputing.
//! - `noshow_reviews`: the sign-offs the gate needs (a fairness review and a clinical safety review of a model
//!   version, by a named person). Nothing is scored or shown without both.
//! - `staff_call_tasks` learns the reason `high_risk`: an appointment above the high threshold goes on the
//!   staff priority list, which a person clears by recording an outcome.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS noshow_assessments ( \
               appointment_id uuid PRIMARY KEY, \
               probability double precision NOT NULL CHECK (probability >= 0 AND probability <= 1), \
               factors jsonb NOT NULL, \
               sample_size integer NOT NULL CHECK (sample_size >= 0), \
               model_version text NOT NULL, \
               computed_at timestamptz NOT NULL, \
               computed_on date NOT NULL)",
            "CREATE TABLE IF NOT EXISTS noshow_reviews ( \
               id uuid PRIMARY KEY, \
               kind text NOT NULL CHECK (kind IN ('fairness','clinical_safety')), \
               reviewer text NOT NULL CHECK (length(btrim(reviewer)) > 0), \
               outcome text NOT NULL CHECK (outcome IN ('approved','rejected')), \
               notes text NOT NULL CHECK (length(btrim(notes)) BETWEEN 1 AND 1000), \
               model_version text NOT NULL, \
               reviewed_at timestamptz NOT NULL DEFAULT now())",
            "CREATE INDEX IF NOT EXISTS idx_noshow_reviews_kind ON noshow_reviews (kind, model_version, reviewed_at DESC)",
            "ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check",
            "ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check \
               CHECK (reason IN ('silent','reschedule_request','urgent_pathway','transport_failed','high_risk'))",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Restoring a narrower CHECK fails, and the down stops, while a row still uses a newer value:
        // a down never deletes or rewrites data to make room.
        let db = manager.get_connection();
        for ddl in [
            "DROP TABLE IF EXISTS noshow_reviews",
            "DROP TABLE IF EXISTS noshow_assessments",
            "ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check",
            "ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check \
               CHECK (reason IN ('silent','reschedule_request','urgent_pathway','transport_failed'))",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }
}
