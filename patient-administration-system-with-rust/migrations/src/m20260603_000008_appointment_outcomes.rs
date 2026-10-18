//! CAP-1 / CAP-2: structured cancellation data on appointments.
//!
//! Until now a miss was just a status. These columns record who cancelled, why (a barrier code),
//! when, and how many whole days of notice the service had, which separates an early release
//! (the slot can be refilled) from a silent miss (it cannot). `confirmed_at` records a patient's
//! "yes, I am coming" reply (CAP-2).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "ALTER TABLE appointments \
               ADD COLUMN IF NOT EXISTS cancelled_at timestamptz NULL, \
               ADD COLUMN IF NOT EXISTS cancelled_by text NULL, \
               ADD COLUMN IF NOT EXISTS barrier_code text NULL, \
               ADD COLUMN IF NOT EXISTS notice_days integer NULL, \
               ADD COLUMN IF NOT EXISTS confirmed_at timestamptz NULL",
            "ALTER TABLE appointments ADD CONSTRAINT appointments_cancelled_by_chk \
               CHECK (cancelled_by IS NULL OR cancelled_by IN ('patient','provider','system'))",
            "ALTER TABLE appointments ADD CONSTRAINT appointments_barrier_code_chk \
               CHECK (barrier_code IS NULL OR barrier_code IN \
               ('cannot_travel','caring','work','feel_better','unwell','forgot','other'))",
            "ALTER TABLE appointments ADD CONSTRAINT appointments_notice_days_chk \
               CHECK (notice_days IS NULL OR notice_days >= 0)",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE appointments \
                   DROP CONSTRAINT IF EXISTS appointments_cancelled_by_chk, \
                   DROP CONSTRAINT IF EXISTS appointments_barrier_code_chk, \
                   DROP CONSTRAINT IF EXISTS appointments_notice_days_chk, \
                   DROP COLUMN IF EXISTS cancelled_at, \
                   DROP COLUMN IF EXISTS cancelled_by, \
                   DROP COLUMN IF EXISTS barrier_code, \
                   DROP COLUMN IF EXISTS notice_days, \
                   DROP COLUMN IF EXISTS confirmed_at",
            )
            .await?;
        Ok(())
    }
}
