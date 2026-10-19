//! CAP-15: non-emergency transport booked from a confirmed appointment.
//!
//! - `transport_eligibility`: staff record that a patient is eligible, and on what basis. The PAS never
//!   infers it.
//! - `transport_consents`: the patient's explicit consent to share their pick-up address with the transport
//!   provider, and the lawful basis recorded. Withdrawing deletes the row (the audit log keeps the history).
//! - `transport_bookings`: one ride per appointment. The pick-up address is NOT stored here: it is read from
//!   person-service at request time and sent to the provider.
//! - `staff_call_tasks` learns the reason `transport_failed`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for ddl in [
            "CREATE TABLE IF NOT EXISTS transport_eligibility ( \
               person_id uuid PRIMARY KEY, \
               eligible_by text NOT NULL CHECK (length(btrim(eligible_by)) > 0), \
               basis text NOT NULL CHECK (length(btrim(basis)) > 0), \
               recorded_at timestamptz NOT NULL DEFAULT now())",
            "CREATE TABLE IF NOT EXISTS transport_consents ( \
               person_id uuid PRIMARY KEY, \
               recorded_by text NOT NULL CHECK (length(btrim(recorded_by)) > 0), \
               basis text NOT NULL CHECK (length(btrim(basis)) > 0), \
               recorded_at timestamptz NOT NULL DEFAULT now())",
            "CREATE TABLE IF NOT EXISTS transport_bookings ( \
               id uuid PRIMARY KEY, \
               appointment_id uuid NOT NULL, \
               person_id uuid NOT NULL, \
               status text NOT NULL CHECK (status IN ('requested','booked','cancelled','failed')), \
               provider_ref text NULL, \
               window_start timestamptz NULL, \
               window_end timestamptz NULL, \
               last_error text NULL, \
               created_at timestamptz NOT NULL DEFAULT now(), \
               updated_at timestamptz NOT NULL DEFAULT now())",
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_transport_active ON transport_bookings (appointment_id) \
               WHERE status IN ('requested','booked')",
            "ALTER TABLE staff_call_tasks DROP CONSTRAINT IF EXISTS staff_call_tasks_reason_check",
            "ALTER TABLE staff_call_tasks ADD CONSTRAINT staff_call_tasks_reason_check \
               CHECK (reason IN ('silent','reschedule_request','urgent_pathway','transport_failed'))",
        ] {
            db.execute_unprepared(ddl).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for t in [
            "transport_bookings",
            "transport_consents",
            "transport_eligibility",
        ] {
            db.execute_unprepared(&format!("DROP TABLE IF EXISTS {t}"))
                .await?;
        }
        Ok(())
    }
}
