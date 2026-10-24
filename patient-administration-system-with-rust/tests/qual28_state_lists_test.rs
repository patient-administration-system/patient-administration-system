//! QUAL-28: the values the code accepts for a state column are exactly the values its CHECK constraint
//! allows. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! A code list that gains a value the database refuses turns a valid request into a 500; a constraint that
//! gains a value the code does not know lets rows in that the code then misreads. Either way this fails,
//! naming the column and the difference. Add a row here for every new state column with a code-side list.

mod common;

use std::collections::BTreeSet;

use patient_administration_system::{
    clinic_templates, clinical_priority, communication, contact, follow_up, identity, interpreter,
    pifu, proxy, receipts, referrals, support, triage, waits,
};
use sea_orm::{ConnectionTrait, DbBackend, Statement};

/// The quoted values in a CHECK constraint, e.g. `status = ANY (ARRAY['a'::text, 'b'::text])` gives {a, b}.
async fn allowed(db: &sea_orm::DatabaseConnection, constraint: &str) -> BTreeSet<String> {
    let def: String = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_get_constraintdef(oid) AS def FROM pg_constraint WHERE conname = $1",
            [constraint.into()],
        ))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no constraint {constraint}"))
        .try_get("", "def")
        .unwrap();
    def.split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn every_code_side_state_list_matches_its_check_constraint() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let cases: &[(&str, &[&str])] = &[
        ("referrals_status_chk", &referrals::STATUSES),
        ("referrals_source_chk", &referrals::SOURCES),
        ("support_flags_kind_check", &support::KINDS),
        ("support_flags_outcome_check", &support::OUTCOMES),
        ("follow_up_tasks_outcome_check", &follow_up::OUTCOMES),
        ("identity_proofings_method_check", &identity::METHODS),
        ("message_receipts_event_check", &receipts::EVENTS),
        ("still_needed_checks_answer_check", &waits::ANSWERS),
        (
            "whatsapp_optins_source_check",
            &communication::whatsapp::SOURCES,
        ),
        (
            "patient_communication_profiles_accessibility_needs_check",
            &contact::ACCESSIBILITY_NEEDS,
        ),
        (
            "patient_communication_profiles_preferred_channel_check",
            &contact::CHANNELS,
        ),
        // Added with the second pass over the lists that already existed in code.
        ("chk_slots_slot_type", &clinic_templates::SLOT_TYPES),
        ("interpreter_requests_kind_check", &interpreter::KINDS),
        ("pifu_requests_channel_check", &pifu::CHANNELS),
        ("proxy_authorities_scopes_check", &proxy::SCOPES),
        ("proxy_authorities_basis_check", &proxy::BASES),
        ("triage_assessments_category_check", &triage::CATEGORIES),
        ("triage_assessments_triage_system_check", &triage::SYSTEMS),
        (
            "waitlist_entries_clinical_priority_chk",
            &clinical_priority::CODES,
        ),
    ];
    let mut drift = Vec::new();
    for (constraint, code) in cases {
        let db = allowed(&state.db, constraint).await;
        let code: BTreeSet<String> = code.iter().map(|s| s.to_string()).collect();
        if db != code {
            drift.push(format!(
                "{constraint}: only in the code {:?}, only in the database {:?}",
                code.difference(&db).collect::<Vec<_>>(),
                db.difference(&code).collect::<Vec<_>>()
            ));
        }
    }
    assert!(drift.is_empty(), "{drift:#?}");
}
