#![cfg_attr(not(test), forbid(unsafe_code))]
pub use sea_orm_migration::prelude::*;

mod m20260520_000001_init;
mod m20260525_000002_outbox_dlq;
mod m20260526_000003_appointment_series;
mod m20260527_000004_coverage;
mod m20260528_000005_patient_replaced_by;
mod m20260601_000006_delegate_identity;
mod m20260602_000007_appointment_series_worker_id;
mod m20260603_000008_appointment_outcomes;
mod m20260604_000009_inbound_replies;
mod m20260605_000010_standby_and_releases;
mod m20260606_000011_communication_profiles;
mod m20260607_000012_barrier_codes;
mod m20260608_000013_inbound_texts;
mod m20260609_000014_message_optouts;
mod m20260610_000015_template_locales;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260520_000001_init::Migration),
            Box::new(m20260525_000002_outbox_dlq::Migration),
            Box::new(m20260526_000003_appointment_series::Migration),
            Box::new(m20260527_000004_coverage::Migration),
            Box::new(m20260528_000005_patient_replaced_by::Migration),
            Box::new(m20260601_000006_delegate_identity::Migration),
            Box::new(m20260602_000007_appointment_series_worker_id::Migration),
            Box::new(m20260603_000008_appointment_outcomes::Migration),
            Box::new(m20260604_000009_inbound_replies::Migration),
            Box::new(m20260605_000010_standby_and_releases::Migration),
            Box::new(m20260606_000011_communication_profiles::Migration),
            Box::new(m20260607_000012_barrier_codes::Migration),
            Box::new(m20260608_000013_inbound_texts::Migration),
            Box::new(m20260609_000014_message_optouts::Migration),
            Box::new(m20260610_000015_template_locales::Migration),
        ]
    }
}
