# Tasks

The live work queue for the monorepo. [`plan.md`](plan.md) holds the research and reasoning behind these tasks.

## Conventions

- **IDs.** One series per area, numbered in order of creation, never reused:
  `BUG` fixed defects, `DEP` dependencies, `OPS` containers and operations, `UI` the SvelteKit UI,
  `REM` appointment reminders, `NOT` patient notifications, `REF` conventions adopted from the reference monorepo,
  `CAS` ideas from care-appointment-success, `DTAC` the NHS DTAC self-assessment
  ([`docs/.../2.0/index.md`](docs/united-kingdom/national-health-service/digital-technology-assessment-criteria/2.0/index.md)), and `REL` release.
- **Sizes.** S is about a day, M a few days, L a week or more.
- **A box is ticked only when** an automated test or a clearly described manual check confirms the acceptance criterion. "Landed" items name that check.
- **Entry shape.** `- [ ] **ID (size), Title.** What and where. **Acceptance:** a testable statement.` Finished entries add **Landed 2026-10-18** and **Deviations, stated plainly**.
- **Three-part changes.** A behavioural change is spec + code + test together (see REF-7).

---

## Landed (2026-10-18)

### Bugs found by running things that had never run

- [x] **BUG-1 (S), `appointment_series.practitioner_id` was never renamed to `worker_id`.** The delegate-identity migration renamed five tables and missed this one, while the entity and the `worker-merged` webhook use `worker_id`. Every appointment-series write and every worker merge failed. Fixed by a guarded migration `m20260602_000007_appointment_series_worker_id`.
  **Acceptance, as tested:** `tests/upstream_merge_webhook_test.rs::test_worker_merged_webhook_happy_path` passes against a migrated Postgres.
- [x] **BUG-2 (S), the Loco front-end could not start.** Four defects: `mailer.smtp` had no `host`; `queue.kind: Null` does not exist in Loco 0.14; Loco's default routes register `/_health` a second time; no Tera view engine initializer, so every page returned 500. Fixed in `config/*.yaml`, `src/app.rs`, `src/controllers/health.rs`, `src/initializers/view_engine.rs`.
  **Acceptance, as tested:** all 13 tests in `patient-administration-system-frontend/tests/dashboard_smoke.rs` pass with `DATABASE_URL` set. **Deviation, stated plainly:** `/_ping` is now this app's own handler because Loco's `ping` module is private.
- [x] **BUG-3 (S), webhook tests raced on process-wide environment variables.** Cargo runs a binary's tests in parallel; the tests set and cleared the webhook secrets. Serialized with a lock.
  **Acceptance, as tested:** `cargo test --workspace` passes three runs in a row with default parallelism.
- [x] **BUG-4 (S), reminders created a pending letter for every channel tried.** An unreachable patient collected junk letters that also blocked later reminders. The recipient is now checked before any letter is generated.
  **Acceptance, as tested:** `tests/reminders_test.rs::reminders_send_once_skip_unreachable_and_ignore_far_future`.
- [x] **BUG-5 (S), Postgres 18 refused the compose volume.** The image wants the volume at `/var/lib/postgresql`, not `.../data`. Fixed in `podman/compose.yml` and `patient-administration-system-with-rust/compose.yml`.
  **Acceptance, as tested:** `podman compose -f podman/compose.yml up -d` reaches "Healthy".
- [x] **BUG-6 (S), the server container image could not build.** The `Containerfile` omitted `templates/` (embedded with `include_str!`) and `curl` (needed by `utoipa-swagger-ui` 10 at build time).
  **Acceptance, as tested:** `podman build --format docker -f Containerfile .` succeeds and the image answers `/api/health` with `database: ok` and serves `/swagger-ui/`.
- [x] **BUG-7 (S), Keycloak 26 intercepted every seeded login.** Users without first and last names are sent to "Update Account Information". Names added to `podman/keycloak/pas-realm.json`.
  **Acceptance, as tested:** `tests/e2e/sign-in.spec.ts` signs in as `patient` and `admin`.

### Dependencies

- [x] **DEP-1 (M), update dependencies.** `cargo update`; `axum` 0.7 to 0.8 (route params `:id` to `{id}`), `utoipa` 5 to 6, `utoipa-swagger-ui` 8 to 10, `tantivy` 0.22 to 0.27, `quick-xml` 0.36 to 0.42.
  **Acceptance, as tested:** `cargo build`, `cargo clippy --workspace --all-targets` (no warnings) and the full test suite.
  **Deviations, stated plainly:** `loco-rs` stays at 0.14 because 1.x needs `sea-orm` 2 across the whole crate (tried, reverted; see DEP-2). `cargo audit` still reports 6 vulnerabilities, all transitive and unfixable here (see DEP-2). No Dependabot alerts or pull requests exist for the repository.

### Containers

- [x] **OPS-1 (M), PostgreSQL and Keycloak under Podman.** [`podman/compose.yml`](podman/compose.yml), Keycloak realm `pas` ([`podman/keycloak/pas-realm.json`](podman/keycloak/pas-realm.json)) with roles `patient`, `staff`, `admin`, a confidential client with PKCE, and mappers that put `person_id` and `roles` in the ID token. Ports are overridable (`PAS_PG_PORT`, `PAS_KEYCLOAK_PORT`).
  **Acceptance, as tested:** stack started with Podman; Keycloak imported the realm and ran on the `keycloak` database; the browser sign-in tests pass against it.
- [x] **OPS-2 (S), Docker to Podman.** `Dockerfile` to `Containerfile`, `docker-compose*.yml` to `compose*.yml`, `.dockerignore` to `.containerignore`, fully qualified image names, docs updated. A `Containerfile` was added for the UI.
  **Acceptance, as tested:** `podman compose config` accepts all three compose files; both images build. **Deviation, stated plainly:** the UI image was built but not started in a container.

### Rename

- [x] **OPS-3 (S), `patient-administration-system-rust-crate` to `patient-administration-system-with-rust`.** Package name unchanged. History preserved with `git mv`; manifests and docs updated; CHANGELOG history left as written.
  **Acceptance, as tested:** workspace builds and tests pass.

### UI

- [x] **UI-1 (L), `patient-administration-system-ui-with-svelte`.** *(2026-10-18: kanban board updated to 0.8.1, which names cards by their full content; see its CHANGELOG.)* SvelteKit 3, Svelte 5, TypeScript 6 strict, Vite 8, adapter-node, pnpm, Node 26, Vitest, Playwright: the reference project's stack. Lily components: `svelte-calendar` (staff calendar, patient agenda), `svelte-kanban-board` (waitlist), `svelte-gantt-chart` (four-week schedule), `svelte-data-grid` (patient search). Backend-for-frontend: tokens and the API bearer stay on the server.
  **Acceptance, as tested:** `pnpm run check` 0 errors; 19 unit tests; 3 browser tests against real Keycloak; signed-out requests redirect to sign-in; strict CSP and security headers present.
  **Deviations, stated plainly:** no i18n or locale-prefixed URLs yet (REF-5); no axe accessibility run (DTAC-12); the staff pages were checked against a mock API returning empty data, not a seeded hospital.
- [x] **UI-2 (M), patient sign-in to see one's own appointments.** `/my/appointments`; the person id comes from the verified ID token, never the URL.
  **Acceptance, as tested:** `sign-in.spec.ts` asserts the API was asked for exactly the token's `person_id`, and that a patient gets 403 on `/appointments` and `/admin/reminders`.
  **Deviation, stated plainly:** nothing proves the person signing in *is* that patient; an administrator links the account by setting `person_id`. See DTAC-9.
- [x] **UI-3 (S), `GET /api/appointments?from&to` and `GET /api/patients/{id}/appointments`.** The API had no way to read appointments.
  **Acceptance, as tested:** the container smoke check returned 200; UI tests exercise both through the mock. **Deviation, stated plainly:** there is no Rust integration test for these two handlers yet (REF-3).

### Reminders and notifications

- [x] **REM-1 (L), appointment reminders.** `ReminderService` scans booked appointments inside a lead window (`PAS_REMINDER_LEAD_HOURS`, default 24, every 5 minutes, opt in with `PAS_REMINDERS_ENABLED=true`), renders the `appointment_reminder` template, and tries the template's channels in order until one reaches the patient. A rendered letter is the "already reminded" marker.
  **Acceptance, as tested:** `tests/reminders_test.rs` (sends once; skips an unreachable patient; ignores appointments outside the window; never double-sends).
- [x] **REM-2 (M), administrator-editable message templates.** `PUT /api/letter-templates/{id}` (Tera syntax checked before saving, audited) and the `/admin/reminders` page (admin role only).
  **Acceptance, as tested:** `reminders_test.rs::edited_template_wording_is_what_gets_sent`; browser test opens the editor as `admin`.
  **Deviation, stated plainly:** saving a template from the browser form was not exercised end to end.
- [x] **NOT-1 (M), notification channels.** Postal letter (`print`), email, SMS, WhatsApp, Apple Messages and the secure portal. Email, WhatsApp and Apple Messages are wired per channel with `PAS_EMAIL_PROVIDER`, `PAS_WHATSAPP_PROVIDER`, `PAS_APPLE_MESSAGES_PROVIDER`, each `log` or off.
  **Deviation, stated plainly:** **only log-only senders exist.** No carrier integration (Twilio, WhatsApp Business, Apple Messages for Business, an SMTP relay) is built, so no real message leaves the system. Postal letters are stored as `pending` and an operator marks them sent; there is no print-and-post integration.
- [x] **NOT-2 (M), secure portal notifications.** Channel `portal`: storing the message delivers it. `{{ portal_url }}` is available to every template (`PAS_PORTAL_URL`) so SMS and email can say "you have a message" without clinical detail. `GET /api/patients/{id}/letters?channel=portal`; patients read them at `/my/messages`.
  **Acceptance, as tested:** `reminders_test.rs::portal_channel_stores_and_delivers_without_a_carrier_and_exposes_portal_url`; the browser test reads a portal message after signing in.

### Documentation

- [x] **DTAC-0 (M), NHS DTAC v2.0 self-assessment.** [`docs/.../2.0/index.md`](docs/united-kingdom/national-health-service/digital-technology-assessment-criteria/2.0/index.md). **Result stated in the document: the product would not pass today.**

---

## Open

### DTAC (from the self-assessment)

- [ ] **DTAC-1 (L), Clinical Safety Officer and DCB0129.** Name a CSO; run the risk management process; write the hazard log and Clinical Safety Case Report. Needs a registered clinician. **Acceptance:** a named CSO and the two documents exist under `governance/`, reviewed by the CSO.
- [ ] **DTAC-2 (M), DPIA, privacy notice and data-flow diagram.** **Acceptance:** documents under `governance/` cover every point listed in checklist C2.2.2.
- [ ] **DTAC-3 (S), `LICENSE` file and `SECURITY.md`.** The manifests declare SPDX licences; no licence file exists. **Acceptance:** a test (as in the reference's `licence_files.rs`) fails if a manifest's licence has no matching file.
- [ ] **DTAC-4 (M), per-user authentication on the API and a trustworthy audit user id.** Today one shared bearer token (or none) and an `X-User-Id` header "recorded as-is". **Acceptance:** the audit user id comes from a verified token claim; a request with a forged header is recorded under the verified identity (test).
- [ ] **DTAC-5 (S), require MFA in the Keycloak realm and write the MFA plan.** **Acceptance:** the realm requires a second factor for `staff` and `admin`; the browser test still signs in (using a stored TOTP secret for the seeded users).
- [ ] **DTAC-6 (M), CI.** See REF-1.
- [ ] **DTAC-7 (M), wire `require_consent` into every handler that needs it.** Marketing and research uses first. **Acceptance:** a test per gated handler returns 403 without active consent.
- [ ] **DTAC-8 (M), NHS number validation and PDS.** Validate the modulus-11 check digit; look up PDS or write the alternative. **Acceptance:** unit tests for the check digit; an integration test against the PDS sandbox or a documented decision.
- [ ] **DTAC-9 (M), identity proofing before a sign-in is linked to a `person_id`.** Assess NHS login. **Acceptance:** an account cannot reach `/my/*` until a staff action records the proofing (audited); test.
- [ ] **DTAC-10 (L), external penetration test, Cyber Essentials, Cyber Security Charter.** Organisational; needs an owner and budget.
- [ ] **DTAC-11 (L), usability test with real patients and staff.** Organisational.
- [ ] **DTAC-12 (M), WCAG 2.2 AA.** axe over every route (enumerated from `src/routes`), keyboard and screen-reader pass, a mobile viewport project, a published accessibility statement. **Acceptance:** the Playwright axe suite reports zero serious or critical violations.

### Reminders and notifications: what is not built

- [ ] **NOT-3 (L), real carrier integrations.** SMTP (email), Twilio or similar (SMS), WhatsApp Business, Apple Messages for Business. Each behind the existing `MessageSender` trait. **Acceptance:** a sandbox send per channel in a test, with secrets read from the environment and never logged.
- [ ] **NOT-4 (L), delivery receipts and retries.** Record delivered, bounced and replied; retry with back-off; move a letter to `failed` after N attempts; surface bounces so contact details can be fixed. **Acceptance:** tests for a receipt webhook and for the retry schedule.
- [ ] **NOT-5 (M), consent, opt-out and quiet hours.** STOP keyword handling, per-channel opt-out, no sends outside permitted hours, and the legal basis recorded per message type (PECR, UK GDPR). Reminders are service messages, not marketing, but the rule must be written down and tested. **Acceptance:** a patient who has opted out of a channel is never sent on it (test); quiet hours defer rather than drop.
- [ ] **NOT-6 (M), patient channel and language preferences.** Store once, use on every channel. **Acceptance:** a patient who prefers WhatsApp and Welsh gets the Welsh template on WhatsApp first.
- [ ] **NOT-7 (M), postal letter print queue.** A list of `print` letters awaiting posting, a batch export for the mail house, and a way to mark a batch sent. **Acceptance:** export then mark-sent flips exactly the exported letters.
- [ ] **REM-3 (L), two-way reminders.** Reply 1 or YES to confirm, 2 to reschedule, 3 or CANCEL to cancel, in English and Welsh; cancelling offers two or three alternative slots within 14 days. **Acceptance:** an inbound-message webhook test per reply word.
- [ ] **REM-4 (M), reminder cadence.** Multiple touches (for example 7 days, 3 days, 24 hours) instead of one. The 3-day touch is the last that leaves staff time to backfill. **Acceptance:** a test that each touch sends once and a cancelled appointment stops the series.
- [ ] **REM-5 (S), the reminder template editor saves from the browser.** **Acceptance:** an end-to-end test edits, saves and reloads a template.

### Capabilities requested 2026-10-18 (CAP)

Eleven capabilities for reducing missed appointments, from the care-appointment-success analysis. Design decisions that apply to all of them:

- **Identity stays upstream.** Demographics remain in `person-service`. Communication preferences, contact reliability and coarse analytics attributes are *PAS-owned* and keyed by `person_id` (`patient_communication_profiles`).
- **Every new column is classified** in `governance/personal-data-inventory.csv` (the test fails otherwise).
- **Nothing sends automatically yet.** Offers, backfill and follow-up create records and events; real delivery is still NOT-3 and consent is still NOT-5. This is deliberate.
- **Support can only be added.** No column, endpoint or query lets a risk indicator reduce a slot, delay a contact or deprioritise anyone (NFR-11); a schema guard test enforces it (CAP-10).

- [x] **CAP-1 (M), structured cancellation reasons and notice.** **Landed 2026-10-18.** Migration `m20260603_000008_appointment_outcomes` (with CHECK constraints) adds `cancelled_at`, `cancelled_by` (patient, provider, system), `barrier_code` (`cannot_travel`, `caring`, `work`, `feel_better`, `unwell`, `forgot`, `other`), `notice_days` and `confirmed_at`. `SchedulingService::cancel_with` records them in one transaction and puts them on the `AppointmentCancelled` outbox event; `PUT /api/appointments/{id}/barrier` adds a barrier afterwards. `release_class` (`early_release` at 3 or more days, `late_release`, `silent_miss`) is derived on read. **Acceptance, as tested:** FR-13 to FR-16 (4 database tests, 4 unit tests); an unknown code gets 422 and changes nothing. **Deviations, stated plainly:** (1) the repository's `create`/`update` returned the caller's struct, not the stored row, which hid the derived field; they now return the stored row. (2) A no-show records no notice (`NULL`), because nobody told us. (3) The UI has no form to enter a barrier yet. (4) The list endpoint returning the new fields is covered through the repository, not an HTTP test.
- [x] **CAP-2 (L), write-back API for confirm, cancel, rebook.** **Landed 2026-10-18.** `POST /api/write-back/replies` and `POST /api/write-back/batch` (JSON array or CSV, at most 1000 rows, each row independent). Migration `m20260604_000009_inbound_replies`; `src/writeback/`; the booking, cancel and a new rebook step were extracted into transaction-level functions so a reply is applied and recorded atomically. **Acceptance, as tested:** FR-17 to FR-22 (9 database tests, 6 unit tests), including 8 simultaneous deliveries of one message applying exactly once. **Deviations, stated plainly:** (1) the first version had a real race (two simultaneous deliveries, one failing); the test found it and an advisory lock on the message id now serializes them. (2) The sender is authenticated only by the API's shared bearer token, like every caller; there is no per-sender signature (DTAC-4). (3) Nothing yet turns an inbound WhatsApp/SMS message into one of these calls; that is the carrier integration, NOT-3. (4) Confirmation does not yet cancel a later reminder or alter the schedule view.
- [x] **CAP-3 (L), released-slot event and standby list.** **Landed 2026-10-18.** Migration `m20260605_000010_standby_and_releases`; `src/standby/`. Freeing a future slot records one `slot_releases` row and publishes `SlotReleased` (carrying notice and class); booking that slot closes it as `refilled` with the seconds taken. `POST/GET /api/standby`, `DELETE /api/standby/{id}`, `POST /api/standby/{id}/book`, `GET /api/slot-releases` (open, refilled, expired, with a summary and median time to refill), `GET /api/slot-releases/{id}/candidates`. **Acceptance, as tested:** FR-28 to FR-33 (8 database tests, 3 unit tests). **Rule, by design and in the module docs:** candidates are ordered by time on standby and nothing else; no score or flag takes part (NFR-11). **Deviations, stated plainly:** (1) nothing contacts the candidates: offering is a person's decision until consent and opt-out exist (NOT-5). (2) Tags are filters a coordinator applies; they do not rank. (3) Only the explicit tag filters exist: a slot has no language of its own, so "language" matches the patient's stated language, not a clinician's. (4) A new inventory category `preference` was added for standby tags.
- [ ] **CAP-4 (L), reliable contact and communication needs.** A validated E.164 mobile and `verified_at`, an alternate or carer contact, preferred channel, preferred language, accessibility needs (easy-read, large print, audio, British Sign Language). Readable by other systems (REST and a FHIR `Patient` fragment). Stale contact details are listed. Reminders honour the preferred channel and the verified number. **Acceptance:** an invalid number is refused; verifying sets `verified_at`; a profile older than N days is listed as stale; a reminder for a patient preferring WhatsApp tries it first.
- [ ] **CAP-5 (M), patient-led rebooking.** `GET /api/appointments/{id}/rebook-options` offers two or three alternative slots (spread across days and parts of the day); the patient chooses one from *that offer* (`POST /api/appointments/{id}/rebook`). **Acceptance:** options never overlap the patient's other appointments; choosing a slot not offered is refused; the old appointment is released (`rescheduled`) and the new one booked atomically.
- [ ] **CAP-6 (M), audit trail and reason codes.** `reason_code` and `context` on audit rows; `trigger` and `trigger_ref` on every generated letter so any contact can be explained; `GET /api/appointments/{id}/history` and `GET /api/patients/{id}/contact-log`. **Acceptance:** a cancel's history shows who, when and why; every reminder records why it was sent.
- [ ] **CAP-7 (M), clinical-urgency flag and human follow-up.** A clinician-approved list of services whose missed appointments must always get a person's follow-up (`follow_up_policies`, with `approved_by`). A miss creates a `follow_up_task`; it can only be closed by a person recording an outcome. **Rule, tested: a miss triggers contact, never discharge or removal from a list.** **Acceptance:** a no-show on a listed service creates an urgent task; a policy without an approver is refused; marking a no-show leaves the waitlist entry and RTT clock untouched.
- [ ] **CAP-8 (M), wait-time and lead-time data.** Per patient: lead time of each appointment (booking to visit) and weeks waited per pathway; a list of long waiters due a "do you still need this?" check; recording the answer. **Acceptance:** lead time is computed from booking time; a pathway over N weeks with no recent check is listed; answering "no longer needed" is recorded and audited (it does not remove the entry by itself).
- [ ] **CAP-9 (M), Welsh and English templates together.** Templates carry a locale and a group; a template group must have every required locale (`PAS_REQUIRED_LOCALES`, default `en-001,cy-001`) before it can be active. Reminders choose the variant from the patient's preferred language and record which was used. **Acceptance:** creating or activating a one-language template is refused; a pair is created atomically; a Welsh-preferring patient gets the Welsh text.
- [ ] **CAP-10 (M), support flags that can only add help.** `needs_support_call` and `navigator_referral` records. **Acceptance:** a schema guard fails if any table has a column that scores, ranks, penalises or suppresses contact (names like `risk_score`, `priority_penalty`, `suppress_contact`); rebooking options and standby candidates are identical with and without support flags.
- [ ] **CAP-11 (M), privacy-safe analytics feed.** Aggregated counts by specialty, deprivation quintile, age band, language, time of day and outcome, with cells below k (default 10) suppressed, no identifiers; JSON and CSV. **Acceptance:** no `person_id` or appointment id appears; a cell under k is suppressed (not shown as zero); totals reconcile with the unsuppressed count.

### Core appointment features (CORE), gap analysis 2026-10-18

Checked against the code, not assumed. **Exists:** booking, cancelling and rescheduling (`book_slot`, `cancel_with`, `rebook`, SIU^S12/S13 over HL7, recurring series); attendance statuses (`arrived`, `fulfilled`, `no_show`); letters, SMS reminders and portal messages; ward and breach dashboards; demographics and registration (delegated on purpose to `person-service`). **Missing or partial:** the eight items below.

- [ ] **CORE-1 (M), referral management.** The `referrals` table and model exist but nothing creates, reads or acts on a referral. Add `POST/GET /api/referrals` (source `gp`, `e-rs`, `internal`; external reference such as an e-RS UBRN; status `received`, `waitlisted`, `booked`, `closed`) and `POST /api/referrals/{id}/book-first-available`, which books the earliest free slot for the target service and links the appointment (`appointments.referral_id`). **Acceptance:** the earliest free slot of the service is booked; with none free the referral stays open and says so; the link is stored. **Not in scope:** the e-RS integration itself needs NHS API access; only the data it would carry.
- [x] **CORE-2 (M), live waiting list.** **Landed 2026-10-18.** Before: nothing changed a waitlist entry when an appointment was booked, cancelled, missed or completed, and `from_waitlist_entry_id` was never set. Now `POST /api/waitlist/{id}/book` books a slot for the entry's patient and marks it `booked`; `SchedulingService::sync_waitlist` (called from cancel, no-show, complete and rebook) returns it to `waiting` on a cancel or a miss, marks it `treated` on completion, carries it through a rebook, and leaves a removed or treated entry alone. It also applies to cancels arriving through the write-back API. **Acceptance, as tested:** FR-23 to FR-27 (8 database tests). **Deviations, stated plainly:** (1) booking *without* naming an entry (`POST /api/slots/{id}/book`) still does not look for a matching waiting-list entry for the patient; only the explicit route links them. (2) A miss returns the patient to `waiting` but does not yet create a follow-up task (CAP-7). (3) The RTT clock is not started, paused or stopped by these events; that remains the separate `/api/rtt/*` calls.
- [ ] **CORE-3 (M), resource and clinic scheduling.** Only per-patient overlap is checked, so a room or clinician can be double-booked through two schedules. Refuse a booking when another busy slot overlaps for the same schedule owner; add `Equipment` as an owner kind. **Acceptance:** two schedules owned by the same room cannot both hold a busy slot at the same time; different owners can. **Not in scope:** one appointment needing several resources at once.
- [ ] **CORE-4 (M), attendance and location.** Record `arrived_at`, `completed_at` and the current room, and list who is in the facility now with their wait. **Acceptance:** check-in and complete stamp the times; moving a patient updates the room and is audited; the in-facility list shows minutes waiting.
- [ ] **CORE-5 (S), letter preview.** `POST /api/letters/preview` renders a letter for a patient without storing or sending it. **Acceptance:** returns the rendered subject and body; writes nothing; reports a missing variable.
- [ ] **CORE-6 (S), emergency contacts.** Covered by the carer or alternate contact in CAP-4 with an emergency flag; demographics stay upstream. **Acceptance:** see CAP-4.
- [ ] **CORE-7 (M), activity and performance reporting.** `GET /api/reports/clinic-activity` (booked, attended, did not attend, cancelled by release class, per day or per schedule), utilization (busy over total slots), lead time and waiting time statistics; JSON and CSV. **Acceptance:** counts reconcile with the underlying rows; utilization is busy over total; an empty range gives zeros, not an error.
- [x] **CORE-8, booking, rescheduling, attendance statuses, communications.** Already present; see the list above.

### Dependencies

- [ ] **DEP-2 (L), `loco-rs` 1.x with `sea-orm` 2.** Clears the `quick-xml` advisories (through Loco's `opendal`). Also: `utoipa-swagger-ui` build-time `zip` (3 advisories; needs `zip` 9), `rsa` (no upstream fix). **Acceptance:** `cargo audit` reports no vulnerabilities, or each remaining one is listed with a reason in `deny.toml` (REF-2).

### Conventions adopted from the reference monorepo

Source: `~/git/project-porfolio-management/project-porfolio-management`. Details in [`plan.md`](plan.md#2-reference-monorepo-conventions).

- [ ] **REF-1 (M), CI with a single gate.** **Status 2026-10-18: workflow written, not yet run on GitHub.** `.github/workflows/ci.yml` (nine jobs and `ci-ok`) parses, `ci-ok` needs every job, and every command it runs was run locally and passes (fmt, clippy `-D warnings`, 393 tests, 32 database tests, `cargo deny`, link check, status check, UI checks and browser tests, restore drill, `--locked` build). The first run found nothing yet because there has been none. **Still owed:** push it, watch the first run, then prove a deliberately broken commit turns `ci-ok` red. One workflow, jobs for Rust (`fmt --check`, `clippy --all-targets --all-features -- -D warnings`, tests), Rust with Postgres (the DB tests), UI (`pnpm run check`, `vitest`, `build`, Playwright with Keycloak), docs (link check), and an aggregate `ci-ok` job that fails if any job failed, was cancelled or was skipped. `permissions: contents: read`; cancel superseded runs. **Acceptance:** a deliberately broken commit on a branch turns `ci-ok` red (HD-2 in the reference never proved this; do).
- [x] **REF-2 (S), supply-chain policy.** **Landed 2026-10-18.** `rust-toolchain.toml` (1.99.1), `rustfmt.toml`, `deny.toml` with every ignore justified, Dependabot weekly and grouped. **Acceptance, as tested:** `cargo deny check` passes (advisories, bans, licences, sources). **Deviations, stated plainly:** the ignore list holds 6 advisories and 3 unmaintained crates, all via `loco-rs` 0.14 (removed by DEP-2); `cargo audit` additionally flags `rsa`, which `cargo tree` shows is never compiled (a false positive for this workspace).
- [x] **REF-3 (M), database tests marked `#[ignore]`.** **Landed 2026-10-18.** 32 database tests are `#[ignore]`d and `database_url()` panics without `DATABASE_URL`. **Acceptance, as tested:** `cargo test` reports `393 passed, 32 ignored`; `cargo test -- --ignored` without a database fails loudly; with one, 32 pass. **Deviation, stated plainly:** Rust tests for `GET /api/appointments` and `GET /api/patients/{id}/appointments` are still missing (FR-11 is marked not yet tested).
- [x] **REF-4 (S), `#![forbid(unsafe_code)]`** **Landed 2026-10-18.** `#![cfg_attr(not(test), forbid(unsafe_code))]` in every crate root. **Acceptance, as tested:** the workspace builds; production code has no `unsafe`. **Deviation, stated plainly:** unit tests still use `unsafe` to set environment variables (edition 2024 makes `set_var` unsafe), hence `not(test)`.
- [x] **REF-5 (L), locales.** **Landed 2026-10-18.** Ten locales (`ar-001`, `cy-001`, `de-001`, `de-de`, `en-001`, `es-001`, `fr-001`, `hi-001`, `zh-001`, `zh-cn`), locale-prefixed URLs, right-to-left Arabic, `lang`/`dir` from the server, a language menu, `@sveltia/i18n`. **Acceptance, as tested:** 96 unit tests including completeness, placeholders, naming and the scan for visible English outside `t()` (self-tested); 10 browser tests in `locales.spec.ts` and the a11y spec across four languages. **Deviations, stated plainly:** **every non-English catalogue is an AI draft with no human review** (`governance/translation-review.md`; Welsh must be reviewed before patients see it); some of the six were first drafted by subagents and then edited by me; Lily widget-internal labels are not translated; dates stay ISO/UTC; no `sitemap.xml` with `hreflang`.
- [x] **REF-6 (M), browser quality gates.** **Landed 2026-10-18.** `a11y.spec.ts` (axe, WCAG 2.0 to 2.2 A/AA, serious and critical, every route enumerated from `src/routes`, four languages), `mobile.spec.ts` (390x844), `security-headers.spec.ts` (headers, strict CSP, zero violations). **Acceptance, as tested:** 21 chromium and 2 mobile tests pass against real Keycloak. **Deviations, stated plainly:** the audit found and fixed three real issues (calendar target size, kanban label-in-name, locale not updating on client navigation); automated checks cover only part of WCAG, so DTAC-12 stays open.
- [x] **REF-7 (M), spec-driven development.** **Landed 2026-10-18.** `spec/` with the 18-chapter skeleton, `agents/spec-driven-development.md`, FR/NFR tables naming their verifying test or saying "not yet tested" (11 of 27 are), `scripts/generate-status.sh`. **Acceptance, as tested:** `--check` passes, and was shown to fail when `tasks.md` changed.
- [x] **REF-8 (M), link checking.** **Landed 2026-10-18.** `lychee.toml`, `scripts/check-links.py` (relative links and heading fragments), the `docs` CI job. **Acceptance, as tested:** the script found 18 broken links (renamed directory, a stale doc); all fixed, now 0 in 83 files. **Deviation, stated plainly:** `lychee` itself is not installed here, so only the Python checker was run.
- [ ] **REF-9 (M), release pipeline.** **Status 2026-10-18: written, never run.** `.github/workflows/release.yml` and `scripts/release-notes.sh` (shown to refuse a tag that does not match the crate version or lacks a CHANGELOG entry). The image, attestation and SBOM jobs have not been exercised. Tag must equal the crate version and have a CHANGELOG entry; CycloneDX SBOMs (Rust and npm); container images with provenance. **Acceptance:** a dry-run tag builds the artefacts.
- [x] **REF-10 (S), repository metadata.** **Landed 2026-10-18.** `LICENSE.md` and `LICENSE/` (five texts) with a test, `SECURITY.md`, `MAINTAINERS.md`, `CODEOWNERS`, `CONTRIBUTING.md`, `GOVERNANCE.md`, `AI_STATEMENT.md`, `INSTALL.md`, `CITATION.cff`, `llms.txt`, `llms.json`. **Acceptance, as tested:** `licence_files` test (3) and the link check. **Deviation, stated plainly:** `SECURITY.md` points to GitHub private reporting, not a personal email, so you may want to add a direct contact.
- [x] **REF-11 (M), `governance/` and `operations/`.** **Landed 2026-10-18.** `governance/` (DPIA, hazard log, data flow, record of processing, retention, accessibility and language statements, translation review, readiness checklist), `operations/` (deployment, runbook, backup and restore), `scripts/backup.sh` and `restore-test.sh`, and `governance/personal-data-inventory.csv` covering all 249 columns. **Acceptance, as tested:** the inventory test fails on an unclassified column (shown by adding one) and on a stale row; the restore drill passed (30 tables, identical counts). **Deviations, stated plainly:** every governance document is an unreviewed DRAFT; retention periods are placeholders; the restore drill ran on a development database, not production.
- [x] **REF-12 (S), pnpm supply-chain settings.** **Landed 2026-10-18.** `minimumReleaseAge: 1440` in the UI's `pnpm-workspace.yaml`. The reference's exclusion list had no minimum behind it, so it did nothing. **Acceptance, as tested:** the project installs; a throwaway project with a 10-year minimum resolved `is-odd` 0.1.0 instead of the latest 3.0.1.

### Ideas from care-appointment-success

Source: `~/git/healthprojects/care-appointment-success`. Rationale in [`plan.md`](plan.md#3-ideas-from-care-appointment-success). These are ideas, not commitments; several rest on vendor or secondary evidence.

- [ ] **CAS-1 (L), Welsh and English parity.** Every patient-facing message exists as a pair; a build step fails if either is missing; replies are accepted in both languages. Test SMS length with Welsh characters (ŵ, ŷ). **Acceptance:** the pair check runs in CI.
- [ ] **CAS-2 (M), barrier check ("Can you make it?").** One tap each: travel, cost, caring, work, health, anxiety, communication, other. Each maps to a signposting message framed as "will not affect your care". **Acceptance:** a barrier reply records a coded reason and triggers the mapped message.
- [ ] **CAS-3 (L), released-slot backfill.** A standby list with preference tags (short-notice OK, needs transport, language, phone only); when a slot is released, offer it to the standby list. Measure refill rate and time from cancellation to rebook. **Acceptance:** cancelling an appointment offers its slot to the first eligible standby patient (test).
- [ ] **CAS-4 (M), coded cancellation and outcome data.** Outcomes: attended, DNA, patient-cancelled with lead time, hospital-cancelled. Reason codes such as "no longer needed", "unwell", "can't get there". **Acceptance:** every cancel path requires a reason code.
- [ ] **CAS-5 (L), interpretable risk score, support only.** A regularised logistic model (or rules, such as two DNAs in 12 months) with a model card. **Hard rule: a score only adds support. It never removes slots, never overbooks, never penalises.** **Acceptance:** a test that no score reduces the slots offered to any patient; a fairness audit by deprivation, age, sex and language before any live use.
- [ ] **CAS-6 (M), KPIs and equity splits.** DNA rate, equity gap between most and least deprived quintiles, language parity, slot reuse, patient ease, staff admin time; split by specialty, site, age, language, access need and channel. **Acceptance:** a dashboard query per KPI over seeded data.
- [ ] **CAS-7 (M), wording experiments.** A/B test message wording with a read-out per deprivation quintile and per language. Be careful that "cost to the NHS" wording can read as blame. **Acceptance:** a message variant is chosen deterministically per appointment and recorded.
- [ ] **CAS-8 (S), welfare contact within 48 hours of a missed appointment**, not a discharge letter; urgent and cancer pathways always get human follow-up. **Acceptance:** a missed urgent appointment creates a task for a person.
- [ ] **CAS-9 (S), contact-detail hygiene.** Flag stale or bounced numbers for fixing; it caps every other tactic. Depends on NOT-4.
- [ ] **CAS-10 (S), accessible formats.** Easy-read, BSL, voice and letter fallbacks for people with no usable mobile, mapped to the Welsh Accessible Communication and Information Standards. **Acceptance:** a patient flagged "letter only" is never sent an SMS.

---

## Release

- [ ] **REL-1 (S), bump versions and publish.** Not done. Requires every test green in CI (REF-1), a CHANGELOG entry, and an explicit decision to publish: a crates.io publish cannot be undone. **Acceptance:** tag equals crate version; release notes exist.
