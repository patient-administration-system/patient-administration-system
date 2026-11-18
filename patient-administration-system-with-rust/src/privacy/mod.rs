//! privacy
//!
//! The masking helpers are domain rules and live in `patient_administration_system_domain::privacy`; this
//! module re-exports them beside the consent reads, which need the database.

pub use patient_administration_system_domain::privacy::*;

pub mod consent;
