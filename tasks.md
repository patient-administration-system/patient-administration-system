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

- [x] **UI-1 (L), `patient-administration-system-ui-with-svelte`.** SvelteKit 3, Svelte 5, TypeScript 6 strict, Vite 8, adapter-node, pnpm, Node 26, Vitest, Playwright: the reference project's stack. Lily components: `svelte-calendar` (staff calendar, patient agenda), `svelte-kanban-board` (waitlist), `svelte-gantt-chart` (four-week schedule), `svelte-data-grid` (patient search). Backend-for-frontend: tokens and the API bearer stay on the server.
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

### Dependencies

- [ ] **DEP-2 (L), `loco-rs` 1.x with `sea-orm` 2.** Clears the `quick-xml` advisories (through Loco's `opendal`). Also: `utoipa-swagger-ui` build-time `zip` (3 advisories; needs `zip` 9), `rsa` (no upstream fix). **Acceptance:** `cargo audit` reports no vulnerabilities, or each remaining one is listed with a reason in `deny.toml` (REF-2).

### Conventions adopted from the reference monorepo

Source: `~/git/project-porfolio-management/project-porfolio-management`. Details in [`plan.md`](plan.md#2-reference-monorepo-conventions).

- [ ] **REF-1 (M), CI with a single gate.** One workflow, jobs for Rust (`fmt --check`, `clippy --all-targets --all-features -- -D warnings`, tests), Rust with Postgres (the DB tests), UI (`pnpm run check`, `vitest`, `build`, Playwright with Keycloak), docs (link check), and an aggregate `ci-ok` job that fails if any job failed, was cancelled or was skipped. `permissions: contents: read`; cancel superseded runs. **Acceptance:** a deliberately broken commit on a branch turns `ci-ok` red (HD-2 in the reference never proved this; do).
- [ ] **REF-2 (S), supply-chain policy.** `rust-toolchain.toml` pinned and asserted equal to the CI toolchain; `deny.toml` (advisories, licence allow-list, only crates.io) with every ignore annotated; Dependabot weekly and grouped for cargo, npm, GitHub Actions and containers. **Acceptance:** `cargo deny check` passes.
- [ ] **REF-3 (M), database tests marked `#[ignore]`.** So plain `cargo test` needs no database and the Postgres job runs them with `-- --ignored`. Today they skip silently when `DATABASE_URL` is unset, which hides that they did not run (this is how BUG-1 and BUG-2 went unnoticed). Add Rust tests for the new appointment endpoints. **Acceptance:** CI shows the DB tests running, by name.
- [ ] **REF-4 (S), `#![forbid(unsafe_code)]`** in every crate, and a rustfmt configuration. **Acceptance:** the build fails if `unsafe` is added.
- [ ] **REF-5 (L), locales.** Locale codes `<language>-<region>`, lowercase, `-001` for world, no bare two-letter codes (`en-001` default, `cy-001`, `ar-001`, `de-001`, `es-001`, `fr-001`, `hi-001`, `zh-001` and the regional ones). Strings via `@sveltia/i18n`; locale-prefixed URLs; right-to-left for Arabic. **Acceptance:** the reference's tests, ported: every locale covers every key; no extra keys; no bare two-letter names; and a template-scanning test that fails on visible English outside `t()`. Welsh is a first-class requirement (see CAS-1); machine translation needs human review.
- [ ] **REF-6 (M), browser quality gates.** axe over every route, a 390x844 mobile project, a security-headers spec asserting the header set and zero CSP violations per locale. **Acceptance:** all run in CI.
- [ ] **REF-7 (M), spec-driven development.** `spec/` with the section skeleton (purpose, scope, stakeholders, glossary, domain model, functional and non-functional requirements, architecture, API, persistence, testing, compliance, tasks, status, roadmap, open questions, references, change control), `agents/spec-driven-development.md` stating "spec, code and test change together", requirement IDs (`NFR-n`) cited in tests, and a generated implementation-status block checked in CI. **Acceptance:** `scripts/generate-status.sh --check` fails when the block is stale.
- [ ] **REF-8 (M), link checking.** `lychee.toml` (offline, fragments) plus the CI job. **Acceptance:** a broken relative link fails CI.
- [ ] **REF-9 (M), release pipeline.** Tag must equal the crate version and have a CHANGELOG entry; CycloneDX SBOMs (Rust and npm); container images with provenance. **Acceptance:** a dry-run tag builds the artefacts.
- [ ] **REF-10 (S), repository metadata.** `CODEOWNERS`, `CITATION.cff`, `llms.txt` and `llms.json`, `MAINTAINERS.md`, `CONTRIBUTING.md`, `GOVERNANCE.md`, `AI_STATEMENT.md`, `INSTALL.md`. **Acceptance:** links resolve (REF-8).
- [ ] **REF-11 (M), `governance/` and `operations/`.** DPIA, data flow, record of processing, retention schedule, a personal-data inventory CSV kept true to the schema by a test, accessibility statement; runbook, deployment, backup and restore with a restore drill run in CI. Feeds DTAC-1, 2 and 12. **Acceptance:** the inventory test fails if a column holding personal data is not listed.
- [ ] **REF-12 (S), pnpm supply-chain settings.** `engine-strict`, a release-age minimum, an allow-list of build scripts. Partly copied already in the UI project; verify the release-age setting is actually enforced (not found in the reference). **Acceptance:** a freshly published package version is refused.

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
