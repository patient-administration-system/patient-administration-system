#![cfg_attr(not(test), forbid(unsafe_code))]
// Production code handles every failure; tests may unwrap (QUAL-12).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
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
mod m20260611_000016_reminder_ladder;
mod m20260612_000017_rebook_offers;
mod m20260613_000018_virtual_visits;
mod m20260614_000019_follow_up;
mod m20260615_000020_audit_reasons;
mod m20260616_000021_support_flags;
mod m20260617_000022_still_needed_checks;
mod m20260618_000023_transport;
mod m20260619_000024_person_demographics;
mod m20260620_000025_slot_offers;
mod m20260621_000026_attendance;
mod m20260622_000027_referral_management;
mod m20260623_000028_print_queue;
mod m20260624_000029_letter_only;
mod m20260625_000030_message_receipts;
mod m20260626_000031_identity_proofing;
mod m20260627_000032_message_experiments;
mod m20260628_000033_noshow_assessments;
mod m20260629_000034_whatsapp_rules;
mod m20260630_000035_deprivation_lookup;
mod m20260630_000036_noshow_rollout;
mod m20260630_000037_inbound_rebook_choice;
mod m20260630_000038_video_failed;
mod m20260701_000039_tolerances;
mod m20260701_000040_tolerance_scopes_and_alerts;
mod m20260702_000041_referral_triage;
mod m20260702_000042_list_validation;
mod m20260704_000043_follow_up_targets;
mod m20260705_000044_needs_review;
mod m20260706_000045_clinical_priority;
mod m20260707_000046_interpreters;
mod m20260708_000047_group_sessions;
mod m20260709_000048_reasonable_adjustments;
mod m20260710_000049_triage_assessments;
mod m20260711_000050_pifu;
mod m20260712_000051_proxy_authority;

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
            Box::new(m20260611_000016_reminder_ladder::Migration),
            Box::new(m20260612_000017_rebook_offers::Migration),
            Box::new(m20260613_000018_virtual_visits::Migration),
            Box::new(m20260614_000019_follow_up::Migration),
            Box::new(m20260615_000020_audit_reasons::Migration),
            Box::new(m20260616_000021_support_flags::Migration),
            Box::new(m20260617_000022_still_needed_checks::Migration),
            Box::new(m20260618_000023_transport::Migration),
            Box::new(m20260619_000024_person_demographics::Migration),
            Box::new(m20260620_000025_slot_offers::Migration),
            Box::new(m20260621_000026_attendance::Migration),
            Box::new(m20260622_000027_referral_management::Migration),
            Box::new(m20260623_000028_print_queue::Migration),
            Box::new(m20260624_000029_letter_only::Migration),
            Box::new(m20260625_000030_message_receipts::Migration),
            Box::new(m20260626_000031_identity_proofing::Migration),
            Box::new(m20260627_000032_message_experiments::Migration),
            Box::new(m20260628_000033_noshow_assessments::Migration),
            Box::new(m20260629_000034_whatsapp_rules::Migration),
            Box::new(m20260630_000035_deprivation_lookup::Migration),
            Box::new(m20260630_000036_noshow_rollout::Migration),
            Box::new(m20260630_000037_inbound_rebook_choice::Migration),
            Box::new(m20260630_000038_video_failed::Migration),
            Box::new(m20260701_000039_tolerances::Migration),
            Box::new(m20260701_000040_tolerance_scopes_and_alerts::Migration),
            Box::new(m20260702_000041_referral_triage::Migration),
            Box::new(m20260702_000042_list_validation::Migration),
            Box::new(m20260704_000043_follow_up_targets::Migration),
            Box::new(m20260705_000044_needs_review::Migration),
            Box::new(m20260706_000045_clinical_priority::Migration),
            Box::new(m20260707_000046_interpreters::Migration),
            Box::new(m20260708_000047_group_sessions::Migration),
            Box::new(m20260709_000048_reasonable_adjustments::Migration),
            Box::new(m20260710_000049_triage_assessments::Migration),
            Box::new(m20260711_000050_pifu::Migration),
            Box::new(m20260712_000051_proxy_authority::Migration),
        ]
    }
}
