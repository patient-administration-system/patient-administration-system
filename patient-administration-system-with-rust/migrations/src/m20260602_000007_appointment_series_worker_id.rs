//! Finish the `practitioner_id` → `worker_id` rename (follow-up to
//! `m20260601_000006_delegate_identity`).
//!
//! That migration renamed the column on `encounters`, `appointments`,
//! `admissions`, `discharges` and `referrals`, but missed
//! `appointment_series`, while the entity and the inbound
//! `worker-merged` webhook both address it as `worker_id`. Result: every
//! appointment-series write and every worker merge failed with "column does
//! not exist". Guarded, so it is a no-op on a database that already has
//! `worker_id`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DO $$ BEGIN \
                   IF EXISTS (SELECT 1 FROM information_schema.columns \
                              WHERE table_schema = current_schema() \
                                AND table_name = 'appointment_series' \
                                AND column_name = 'practitioner_id') \
                   AND NOT EXISTS (SELECT 1 FROM information_schema.columns \
                              WHERE table_schema = current_schema() \
                                AND table_name = 'appointment_series' \
                                AND column_name = 'worker_id') THEN \
                     ALTER TABLE appointment_series RENAME COLUMN practitioner_id TO worker_id; \
                   END IF; \
                 END $$;",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DO $$ BEGIN \
                   IF EXISTS (SELECT 1 FROM information_schema.columns \
                              WHERE table_schema = current_schema() \
                                AND table_name = 'appointment_series' \
                                AND column_name = 'worker_id') THEN \
                     ALTER TABLE appointment_series RENAME COLUMN worker_id TO practitioner_id; \
                   END IF; \
                 END $$;",
            )
            .await?;
        Ok(())
    }
}
