# 6. Functional requirements

"Verified by" names the automated test. **Not yet tested** is a debt, not a pass.

| ID | Requirement | Verified by |
|---|---|---|
| FR-1 | An appointment MUST NOT move to its own status or out of a terminal status. | unit tests in `patient-administration-system-with-rust/src/models/appointment.rs` |
| FR-2 | A reminder MUST be sent at most once per appointment and template. | `tests/reminders_test.rs::reminders_send_once_skip_unreachable_and_ignore_far_future` |
| FR-3 | A reminder MUST try the template's channels in order and stop at the first that reaches the patient. | **Not yet tested** for the fall-through to a second channel |
| FR-4 | A channel with no contact point for the patient MUST be skipped before any letter is written. | `tests/reminders_test.rs` (unreachable patient has zero letters) |
| FR-5 | Reminders MUST only cover booked appointments starting inside the lead window. | `tests/reminders_test.rs` (far-future appointment ignored) |
| FR-6 | An administrator MUST be able to change a template's subject, body, channels and active flag, and the next reminder MUST use the new wording. | `tests/reminders_test.rs::edited_template_wording_is_what_gets_sent` (API); browser save **not yet tested** |
| FR-7 | A template with invalid Tera syntax MUST be refused when saved. | **Not yet tested** |
| FR-8 | The `portal` channel MUST store and deliver without a carrier, and expose `portal_url` to templates. | `tests/reminders_test.rs::portal_channel_stores_and_delivers_without_a_carrier_and_exposes_portal_url` |
| FR-9 | A patient MUST be able to sign in and see their own appointments and portal messages. | `patient-administration-system-ui-with-svelte/tests/e2e/sign-in.spec.ts` |
| FR-10 | Staff MUST see appointments on a calendar, the waitlist on a board, the next four weeks on a schedule chart, and be able to search patients. | **Not yet tested** against seeded data (pages load against a mock) |
| FR-11 | `GET /api/appointments` MUST reject a range over 366 days or with `to <= from`. | **Not yet tested** |
| FR-12 | A worker merge MUST rewrite every `worker_id` column, including appointment series. | `tests/upstream_merge_webhook_test.rs::test_worker_merged_webhook_happy_path` |
| FR-13 | Cancelling MUST record who cancelled, when, the patient's barrier code if given, and the whole days of notice (never negative). | `tests/cap1_outcomes_test.rs::cancel_records_who_why_when_and_notice_and_is_an_early_release`; unit tests in `src/models/appointment.rs` |
| FR-14 | A cancellation with at least 3 days' notice MUST be an `early_release`, less a `late_release`, and a no-show a `silent_miss`; the class is derived, never stored. | `cap1_outcomes_test.rs::a_cancel_hours_before_is_a_late_release_and_a_no_show_is_a_silent_miss`; unit tests |
| FR-15 | A barrier MUST be recordable after a cancellation or a miss, and MUST be refused for an appointment still to happen. | `cap1_outcomes_test.rs::a_barrier_can_be_added_after_a_miss_but_not_to_a_future_appointment` |
| FR-16 | `POST /api/appointments/{id}/cancel` MUST refuse an unknown barrier code without changing the appointment, and MUST still accept the old free-text body. | `cap1_outcomes_test.rs::the_cancel_endpoint_takes_codes_refuses_unknown_ones_and_still_accepts_the_old_body` |
