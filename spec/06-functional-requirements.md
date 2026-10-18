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
