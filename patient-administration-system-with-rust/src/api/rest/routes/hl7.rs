//! Part of [`super`]: hl7.

use super::*;

/// HL7 v2 messages over HTTP (the MLLP listener posts to these paths).
pub(super) fn hl7v2() -> Router<AppState> {
    Router::new()
        // HL7 v2: parse, then ADT A28/A01/A02/A03. Each handler
        // dedup-or-creates the person upstream via `PersonClient` before
        // invoking the local ADT service. The other ADT variants, DFT^P03,
        // SIU and MFN follow below.
        .route("/api/hl7/v2/parse", post(handlers_hl7v2::parse))
        .route("/api/hl7/v2/patient", post(handlers_hl7v2::add_person))
        .route("/api/hl7/v2/admit", post(handlers_hl7v2::admit))
        .route("/api/hl7/v2/transfer", post(handlers_hl7v2::transfer))
        .route("/api/hl7/v2/discharge", post(handlers_hl7v2::discharge))
        // The remaining ADT triggers (URLs match
        // `listener.rs::route_for_payload` so MLLP traffic reaches
        // them unchanged).
        .route("/api/hl7/v2/register", post(handlers_hl7v2::register))
        .route("/api/hl7/v2/pre-admit", post(handlers_hl7v2::pre_admit))
        .route(
            "/api/hl7/v2/cancel-pre-admit",
            post(handlers_hl7v2::cancel_pre_admit),
        )
        .route(
            "/api/hl7/v2/change-to-inpatient",
            post(handlers_hl7v2::change_to_inpatient),
        )
        .route(
            "/api/hl7/v2/change-to-outpatient",
            post(handlers_hl7v2::change_to_outpatient),
        )
        .route("/api/hl7/v2/update", post(handlers_hl7v2::update))
        .route(
            "/api/hl7/v2/cancel-admit",
            post(handlers_hl7v2::cancel_admit),
        )
        .route(
            "/api/hl7/v2/cancel-transfer",
            post(handlers_hl7v2::cancel_transfer),
        )
        .route(
            "/api/hl7/v2/cancel-discharge",
            post(handlers_hl7v2::cancel_discharge),
        )
        .route("/api/hl7/v2/leave-start", post(handlers_hl7v2::leave_start))
        .route("/api/hl7/v2/leave-end", post(handlers_hl7v2::leave_end))
        .route(
            "/api/hl7/v2/delete-patient",
            post(handlers_hl7v2::delete_patient),
        )
        .route("/api/hl7/v2/merge", post(handlers_hl7v2::merge))
        // DFT / SIU / MFN / batch envelope.
        .route("/api/hl7/v2/dft", post(handlers_hl7v2::dft))
        .route(
            "/api/hl7/v2/schedule-book",
            post(handlers_hl7v2::schedule_book),
        )
        .route(
            "/api/hl7/v2/schedule-reschedule",
            post(handlers_hl7v2::schedule_reschedule),
        )
        .route(
            "/api/hl7/v2/schedule-modify",
            post(handlers_hl7v2::schedule_modify),
        )
        .route(
            "/api/hl7/v2/schedule-cancel",
            post(handlers_hl7v2::schedule_cancel),
        )
        .route("/api/hl7/v2/mfn-staff", post(handlers_hl7v2::mfn_staff))
        .route(
            "/api/hl7/v2/mfn-location",
            post(handlers_hl7v2::mfn_location),
        )
        .route("/api/hl7/v2/batch", post(handlers_hl7v2::batch))
}
