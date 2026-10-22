//! Part of [`super`]: messaging waiting.

use super::*;

/// Letters, templates, replies, links, texts, receipts, printing and contact.
pub(super) fn messaging() -> Router<AppState> {
    Router::new()
        .route(
            "/api/communication-needs/due-review",
            get(handlers_needs_review::due_review),
        )
        .route(
            "/api/communication-profiles/stale",
            get(handlers_contact::stale_profiles),
        )
        // Write-back (CAP-2): a patient's reply changes the appointment now.
        .route(
            "/api/write-back/replies",
            post(handlers_writeback::post_reply),
        )
        .route(
            "/api/write-back/batch",
            post(handlers_writeback::post_batch),
        )
        .route("/api/reply-links/{token}", get(handlers_links::get_link))
        .route(
            "/api/reply-links/{token}/rebook",
            post(handlers_links::post_link_rebook),
        )
        .route(
            "/api/reply-links/{token}/{action}",
            post(handlers_links::post_link_action),
        )
        .route(
            "/api/staff-call-tasks/open",
            get(handlers_links::open_calls),
        )
        .route(
            "/api/staff-call-tasks/{id}/outcome",
            post(handlers_links::call_outcome),
        )
        .route(
            "/api/letter-template-groups",
            post(handlers_letters::create_group),
        )
        .route(
            "/api/letter-templates/check-sms",
            post(handlers_letters::check_sms),
        )
        .route("/api/write-back/texts", post(handlers_writeback::post_text))
        .route(
            "/api/inbound-texts/open",
            get(handlers_writeback::open_texts),
        )
        .route(
            "/api/inbound-texts/{id}/handled",
            post(handlers_writeback::text_handled),
        )
        .route("/api/offer-links/{token}", get(handlers_offers::get_offer))
        .route(
            "/api/offer-links/{token}/{action}",
            post(handlers_offers::post_offer_action),
        )
        .route("/api/print-queue", get(handlers_print::queue))
        .route("/api/print-batches", post(handlers_print::export))
        .route(
            "/api/print-batches/{id}/sent",
            post(handlers_print::mark_sent),
        )
        .route(
            "/api/write-back/receipts",
            post(handlers_receipts::post_receipt),
        )
        .route(
            "/api/letters/{id}/receipts",
            get(handlers_receipts::letter_receipts),
        )
        .route(
            "/api/contact-problems",
            get(handlers_receipts::contact_problems),
        )
        .route(
            "/api/contact-problems/{person_id}/resolve",
            post(handlers_receipts::resolve_problem),
        )
        .route(
            "/api/letter-templates/{id}/whatsapp-approval",
            put(handlers_whatsapp::approve_template).delete(handlers_whatsapp::revoke_template),
        )
        .route("/api/write-back/ivr/menu", post(handlers_ivr::menu))
        .route("/api/write-back/ivr/press", post(handlers_ivr::press))
        // Letter templates + generation.
        .route(
            "/api/letter-templates",
            post(handlers_letters::create_template).get(handlers_letters::list_templates),
        )
        .route(
            "/api/letter-templates/{id}",
            get(handlers_letters::get_template).put(handlers_letters::update_template),
        )
        .route(
            "/api/letters/preview",
            post(handlers_letters::preview_letter),
        )
        .route(
            "/api/letters/generate",
            post(handlers_letters::generate_letter),
        )
        .route("/api/letters/{id}", get(handlers_letters::get_letter))
        .route("/api/letters/{id}/sent", post(handlers_letters::mark_sent))
        .route(
            "/api/letters/{id}/failed",
            post(handlers_letters::mark_failed),
        )
}

/// Waiting lists, RTT, standby, released slots, referrals and long waits.
pub(super) fn waiting() -> Router<AppState> {
    Router::new()
        .route(
            "/api/waitlist/{id}/clinical-priority",
            put(handlers_clinical_priority::set_priority),
        )
        .route(
            "/api/clinical-priority-reviews/overdue",
            get(handlers_clinical_priority::overdue),
        )
        .route(
            "/api/validation-campaigns",
            post(handlers_validation::start).get(handlers_validation::list),
        )
        .route(
            "/api/validation-campaigns/{id}",
            get(handlers_validation::get),
        )
        .route(
            "/api/validation-campaigns/{id}/items",
            get(handlers_validation::items),
        )
        .route(
            "/api/validation-campaigns/{id}/items/{item}/asked",
            post(handlers_validation::asked),
        )
        .route(
            "/api/validation-campaigns/{id}/items/{item}/answer",
            post(handlers_validation::answer),
        )
        .route(
            "/api/validation-campaigns/{id}/items/{item}/letter-sent",
            post(handlers_validation::letter_sent),
        )
        .route(
            "/api/validation-campaigns/{id}/items/{item}/decision",
            post(handlers_validation::decision),
        )
        .route(
            "/api/validation-campaigns/{id}/items/{item}/reinstate",
            post(handlers_validation::reinstate),
        )
        .route(
            "/api/reports/validation-removals",
            get(handlers_validation::removals_report),
        )
        // Released slots and the standby list (CAP-3).
        .route(
            "/api/standby",
            post(handlers_standby::add_standby).get(handlers_standby::list_standby),
        )
        .route(
            "/api/standby/{id}",
            delete(handlers_standby::remove_standby),
        )
        .route(
            "/api/standby/{id}/book",
            post(handlers_standby::book_from_standby),
        )
        .route("/api/slot-releases", get(handlers_standby::list_releases))
        .route(
            "/api/slot-releases/{id}/candidates",
            get(handlers_standby::release_candidates),
        )
        .route(
            "/api/pathways/due-check",
            get(handlers_waits::due_for_check),
        )
        .route(
            "/api/pathways/{id}/still-needed-checks",
            post(handlers_waits::ask),
        )
        .route(
            "/api/still-needed-checks/{id}/answer",
            post(handlers_waits::answer),
        )
        .route(
            "/api/slot-releases/{id}/offers",
            get(handlers_offers::release_offers),
        )
        .route(
            "/api/slot-releases/{id}/offer",
            post(handlers_offers::offer_now),
        )
        .route(
            "/api/referrals",
            post(handlers_referrals::create).get(handlers_referrals::list),
        )
        .route("/api/referrals/{id}", get(handlers_referrals::get))
        .route(
            "/api/referrals/{id}/book-first-available",
            post(handlers_referrals::book_first_available),
        )
        .route(
            "/api/referrals/{id}/waitlist",
            post(handlers_referrals::waitlist),
        )
        .route("/api/referrals/{id}/close", post(handlers_referrals::close))
        .route(
            "/api/referrals/{id}/triage",
            post(handlers_referrals::triage),
        )
        // Waitlist + RTT.
        .route(
            "/api/waitlist",
            post(handlers_waitlist::add_to_waitlist).get(handlers_waitlist::list_waitlist),
        )
        .route(
            "/api/waitlist/{id}",
            delete(handlers_waitlist::remove_from_waitlist),
        )
        .route(
            "/api/waitlist/{id}/book",
            post(handlers_waitlist::book_from_waitlist),
        )
        .route(
            "/api/waitlist/{id}/follow-up-target",
            put(handlers_follow_up_delay::set_target),
        )
        .route("/api/rtt/start", post(handlers_waitlist::rtt_start))
        .route("/api/rtt/{id}/pause", post(handlers_waitlist::rtt_pause))
        .route("/api/rtt/{id}/resume", post(handlers_waitlist::rtt_resume))
        .route("/api/rtt/{id}/stop", post(handlers_waitlist::rtt_stop))
        .route(
            "/api/rtt/{id}/weeks-waiting",
            get(handlers_waitlist::rtt_weeks_waiting),
        )
        .route(
            "/api/rtt/{id}/breaching",
            get(handlers_waitlist::rtt_is_breaching),
        )
}
