# Plan

Research and reasoning behind [`tasks.md`](tasks.md). Tasks are the commitments; this file is where ideas are weighed. Written 2026-10-18; section 1 brought up to date 2026-10-21; sections 7 to 9 added 2026-10-21 (the research in sections 2 to 6 is as it was written).

## 1. Where the monorepo stands

| Part | What it is | State (2026-10-21) |
|---|---|---|
| `patient-administration-system-with-rust` | Axum 0.8 + SeaORM 2.0 + Postgres 18 server: ADT, appointments, waitlist and RTT, wards and beds, letters, billing, consent, audit, outbox, FHIR R5, HL7 v2, the reminder ladder, replies, links, offers, virtual visits, transport requests, analytics, the no-show score. Identity and workforce are delegated over HTTP | Built; database tests run in CI and on a fresh database; chaos tests; 40 migrations; tolerances (TOL-1). The no-show score is off until named people approve it |
| `patient-administration-system-frontend` | Loco 1.2 server-rendered operator pages (Tera + HTMX) | Starts and passes its smoke tests on an empty database; the ward and RTT pages read a table that no longer exists (BUG-8) |
| `patient-administration-system-ui-with-svelte` | SvelteKit 3 backend-for-frontend: Keycloak sign-in with one-time codes for staff, patient and staff pages, template editor, history and long-wait views | Built; 10 locales; axe, keyboard, reflow, text-spacing and chaos browser tests |
| `podman/` | PostgreSQL 18, Keycloak 26.8 and Redis 8 for development | Working; ports overridable |
| `docs/…/digital-technology-assessment-criteria/2.0` | NHS DTAC v2.0 self-assessment | Still says the product would not pass today: the gaps left are people and contracts (see `tasks.md`) |

**The most useful discovery of this round is a pattern, not a bug.** Four separate defects (BUG-1, BUG-2, BUG-3, BUG-5) had been invisible because the database tests skip silently when `DATABASE_URL` is unset, and `cargo test` therefore looked green. Making those tests impossible to skip in CI (REF-1, REF-3) is worth more than any single fix.

## 2. Reference monorepo conventions

Surveyed from `~/git/project-porfolio-management/project-porfolio-management` (the Rust service, matcher, SvelteKit UI and skill). Adopt the habits that have teeth; skip the volume.

**Spec-driven development.** One `spec/index.md` per subproject (and split `spec/NN-*.md` at the root) with a fixed skeleton, §1 to §18. The rules, in the reference's words: the specification is the canonical artefact; code conforms to it; a behavioural change is one pull request containing spec, code and test; when spec and code disagree, open a task rather than silently rewriting either. Requirement IDs (`NFR-n`) are cited in test files. A generated "implementation status" block is checked in CI, so counts of routes, tests and tasks cannot rot. Spec style: RFC 2119 words, Mermaid not images, one concept per section, split near 300 lines. → REF-7.

**Bulletproofing that has teeth.**

- One aggregate `ci-ok` job is the only required check, and it fails on failure, cancellation *and* skipping. → REF-1.
- The toolchain is pinned and CI asserts the pin matches. `cargo fmt --check` and `clippy -D warnings`. `#![forbid(unsafe_code)]`. → REF-2, REF-4.
- `cargo-deny` with every advisory ignore annotated with a reason. → REF-2.
- Database tests are `#[ignore]` and run in a Postgres job, one binary at a time, so a missing database cannot masquerade as success. → REF-3.
- Browser tests run axe over every route (enumerated from the route tree), plus a phone-width project, plus a spec that asserts the security headers and zero CSP violations per locale. → REF-6.
- Offline link checking (`lychee`) with a standard-library fallback. → REF-8.
- Release verification: tag equals crate version equals a CHANGELOG entry; CycloneDX SBOMs; image provenance. → REF-9.
- Governance files kept true by tests: a personal-data inventory CSV is checked against the database schema, so a new personal-data column fails the build until it is documented. → REF-11. This is the single most transferable idea for a hospital system.
- Candid limits: the reference admits it never pushed a deliberately broken commit to prove each CI job fails. Do that (REF-1 acceptance).

**Locales.** Two separate things share the word.

1. *UI strings.* Codes are `<language>-<region>`, lowercase, hyphenated, `-001` meaning UN M.49 "world", and no bare two-letter codes anywhere: `ar-001` (the only right-to-left one), `cy-001`, `de-001`, `de-de`, `en-001` (default and source of truth), `es-001`, `fr-001`, `hi-001`, `zh-001`, `zh-cn`. Strings are dotted keys with `{name}` placeholders, consumed by `@sveltia/i18n`; URLs are always locale-prefixed; a bare URL is resolved from a cookie, then `Accept-Language`, then the default. The strongest test is one that parses every Svelte file and fails on visible English outside `t()`.
2. *Content trees* (`locales/<code>/for/<role>/index.md`), not needed here.

For a Welsh health board, `cy-001` is not optional (see CAS-1). Machine translation is a draft; a first-language reviewer signs off, and the review is recorded.

**Stack parity.** Rust 2024 edition on a pinned toolchain with Loco, Axum 0.8, SeaORM; SvelteKit 3, Svelte 5 runes, TypeScript 6 strict with `noUncheckedIndexedAccess`, Vite 8, pnpm, Node pinned, Vitest 5, Playwright. The new UI follows this already. The Rust side is a version behind (Loco 0.14, SeaORM 1.1): see DEP-2.

**Metadata worth having.** `CODEOWNERS`, `CITATION.cff`, `llms.txt` and `llms.json`, `AI_STATEMENT.md`, `SECURITY.md`, `GOVERNANCE.md`. → REF-10.

**Not copied, deliberately.** The reference's ten-locale catalogue (about 4,000 lines in one file), ZAP baseline scanning, load tests and fuzzing are good but belong after the basics. They are tracked only as follow-ups inside REF-5, REF-6 and REF-1.

## 3. Ideas from care-appointment-success

Source: `~/git/healthprojects/care-appointment-success`. It is a **bid response** (Contracts for Innovation Cymru, Cwm Taf Morgannwg UHB, about 64,000 missed appointments a year, 8.6% DNA rate, about £150 per miss), not a built product. Its sibling UI is a prototype on fictional data; its sibling service is an untouched Loco starter whose only reusable piece is a Keycloak JWT extractor. So the value is in the thinking, which is substantial, and in its caveats.

**What to take.**

- **Reminders are the baseline, not the innovation.** The evidence for plain reminders is modest (Cochrane rates it low to moderate quality). Differentiate on matching the barrier, letting the patient choose the replacement slot, equity, and refilling released slots. → REM-3, CAS-2, CAS-3.
- **Timing is a lever.** A cadence of 7 days, 3 days and 24 hours; the 3-day touch is the last that leaves staff time to backfill. Transport prompts go earlier (about 10 days). → REM-4.
- **Make every reminder two-way with one action.** Reply 1 or YES to confirm, 2 to reschedule, 3 or CANCEL to cancel; accept Welsh and English words. Cancelling by reply removes the phone hold time that deters people from cancelling, and should immediately offer two or three slots. → REM-3.
- **Optimise early cancellation, not only attendance.** A slot released late is a wasted slot. Measure the refill rate of slots released 72 hours or more ahead. → CAS-3, CAS-6.
- **Backfill with a standby list** carrying preference tags (short notice OK, needs transport, language, phone only). → CAS-3.
- **A barrier check** ("Can you make it?") with one tap per barrier, each mapped to signposting, framed as "this will not affect your care". → CAS-2.
- **Contact hygiene first.** Stale or bounced numbers cap every other tactic. → NOT-4, CAS-9.
- **Fallbacks for people with no usable mobile.** Voice, letter, easy-read, BSL, then a navigator call. → CAS-10, NOT-7.
- **Welfare contact, not a discharge letter,** within 48 hours of a missed appointment; urgent and cancer pathways always get a human. Repeated non-attendance can be a clinical warning sign, not waste. → CAS-8.
- **Content as pairs.** Every message exists in Welsh and English; a build step fails when one is missing. → CAS-1.
- **Data worth capturing from the start.** Outcome (attended, DNA, patient-cancelled with lead time, hospital-cancelled), cancel reason codes, contact outcome (delivered, bounced, replied), and who refilled a slot and how fast. → CAS-4, NOT-4.
- **KPIs with equity splits,** not just an overall DNA rate: the gap between most and least deprived quintiles, language parity, slot reuse, staff time. → CAS-6.

**The equity guardrail (non-negotiable).** A risk score may *only add support*. It must never remove slots, overbook or penalise. The source cites evidence that race-blind no-show models combined with overbooking made one group wait about 30% longer, and warns against steering "unreliable" slots (late afternoon, Mondays, Fridays) to deprived patients, which creates a feedback loop. Overbooking is rejected outright. Make "no score reduces offered slots" a test, and audit calibration and false-negative rate by deprivation, age, sex and language before any live use. → CAS-5. A risk score also re-opens DTAC C1.1.1 (medical device status) and the clinical safety case (DTAC-1).

**Lessons and cautions from that project.**

- Several headline figures (navigator effect about 45%, a vendor's 48% to 16%) are secondary or vendor-reported. Treat them as hypotheses, and label them so in any document we write.
- Booking lead time may be the strongest predictor of a missed appointment, which makes it a *lever* (shorter notice, patient-chosen slots) as much as a model input.
- Its own risk list is worth borrowing as a pre-mortem: data never arrives; Welsh content lags English; the model is accurate but unfair; an automated message cancels an urgent appointment.
- It found nothing on STOP handling, PECR, quiet hours or delivery receipts. Those gaps are ours to fill (NOT-5, NOT-4), and are the likeliest source of a real-world incident.
- Its `plan.md` / `tasks.md` shape (numbered research sections, KPIs, a risk table, phased checklists whose tasks cite the plan section that produced them) is the pattern used here.

## 4. Patient notifications: design

**Channels.** `print` (postal letter), `email`, `sms`, `whatsapp`, `apple_messages`, `portal`. Each is a `DeliveryChannel`; each non-print, non-portal channel has a `MessageSender`. A template lists its channels in preference order; a reminder tries them in order and stops at the first that reaches the patient. A channel is skipped *before* any record is written when the patient has no contact point for it (the BUG-4 lesson).

**Secure portal URL.** The portal channel exists so SMS, WhatsApp and email can carry almost nothing: "You have a new message: `{{ portal_url }}`". The detail is shown only after sign-in, to its owner. This is the privacy-preserving default, because a text message to a shared or lost phone is a disclosure. Templates for open channels should avoid clinical detail and prefer the link.

**What is real and what is not.** The orchestration, templates, audit, deduplication and the portal are real and tested. **No carrier is integrated:** the senders log. Until NOT-3 lands, nothing leaves the system, which is the safe default.

**Template editing.** Administrators edit subject, body, channels and the active flag; Tera syntax is validated before saving; every edit is audited with before and after. Open question: should a template change need a second person's approval before it goes live? A wording error sent to thousands is not recoverable.

## 5. Open questions

1. **Who is the Clinical Safety Officer?** Everything in DTAC C1 waits on a named, registered clinician. (DTAC-1)
2. **Where does patient identity come from?** Linking a Keycloak account to a `person_id` is a manual attribute today with no proofing. NHS login would solve identity proofing for England; Wales and Scotland differ. Decide before real patients sign in. (DTAC-9)
3. **Is the API single-token model acceptable anywhere real?** It is a trusted-caller design. The audit user id is whatever the caller says. (DTAC-4)
4. **`loco-rs` 1.x** needs `sea-orm` 2 across the crate. Worth doing as its own change, with the database tests running in CI first (REF-3), so the migration is protected. (DEP-2)
5. **Reminder consent basis.** Reminders are service messages, but the written position, per channel, belongs in the DPIA before WhatsApp or Apple Messages are switched on. (NOT-5, DTAC-2)
6. **Template approval.** One editor or two? (§4)
7. **Which carrier per channel?** NHS-approved suppliers may constrain this (NHS Notify exists for England; check availability and terms before building a bespoke integration).
8. **Publishing.** `REL-1` asks for a version bump and a crates.io publish. That cannot be undone, and the product does not pass its own self-assessment. The recommendation is to publish only after CI exists and is green.

## 6. Suggested order

1. **Make failures visible:** REF-1, REF-3, REF-2, REF-4. Small, and they protect everything after.
2. **Make it honest to run:** DTAC-3, DTAC-4, DTAC-5 (licence, security policy, real authentication, MFA).
3. **Make notifications real, safely:** NOT-5 (consent and opt-out) before NOT-3 (carriers), then NOT-4 (receipts), REM-4, REM-3.
4. **Make it accessible and multilingual:** DTAC-12, REF-5, CAS-1.
5. **Documents that need people:** DTAC-1, DTAC-2, DTAC-10, DTAC-11 (start early, they have the longest lead time).
6. **Then the improvements that need data:** CAS-4, CAS-6, CAS-3, and only after a fairness audit, CAS-5.
7. **Housekeeping and spec:** REF-7 to REF-12, DEP-2.

## 7. New capabilities: research 2026-10-21

Web research on what a top-tier outpatient PAS for NHS Wales has that this one lacks, checked against the repository (nothing was found for PIFU, interpreter booking, reasonable adjustments, FHIR Subscriptions, AuditEvent, UK Core, BaRS, clinic templates or demand and capacity). Candidates, most valuable first; IDs are in `tasks.md` (NEW-1 to NEW-14). These are candidates, not commitments, and the evidence caveats are stated.

**The finding that changes the most: Wales exchanges FHIR R4, and this PAS speaks only R5.** DHCW's FHIR Standards Wales Implementation Guide (v2.5.0 STU2 on Simplifier; the newest package found is 2.4.0, so pin deliberately) is built on UK Core STU2 and has a `DataStandardsWales-Appointment` profile with a clinic-code extension. UK Core Appointment is R4 too (1.4.0, January 2026). Without an R4 layer nothing from the PAS reaches the National Data Resource or the NHS Wales App. Keep R5 inside; add R4 at the edge. → NEW-1. Sources: <https://simplifier.net/guide/fhir-standards-wales-implementation-guide>, <https://simplifier.net/guide/UK-Core-Implementation-Guide-STU2/Home/ProfilesandExtensions/Profile-UKCore-Appointment>.

**Buildable PAS-side now.**

- **Waiting-list validation campaigns.** CAP-8 has one person asking one patient. Health boards (Swansea Bay, Betsi Cadwaladr) validate in batches: a cohort, a text, a letter for anyone with no mobile or no reply, a deadline; removal confirmed by a person, never automatic; a letter to patient and referrer; reinstatement on request; validation removals reported separately from treatment. The Office for Statistics Regulation criticised hidden "unreported removals" in July 2026, so the separate reporting is the point. → NEW-2. Sources: <https://bcuhb.nhs.wales/patients-visitors/were-getting-in-touch-while-you-wait-for-your-appointment>, <https://www.england.nhs.uk/wp-content/uploads/2022/12/B2121ii-validation-toolkit-and-guidance-december-2022.pdf>, <https://osr.statisticsauthority.gov.uk/wp-content/uploads/2026/07/Annex-Waiting-Lists-and-Unreported-Removals-2.pdf>.
- **Patient-initiated follow-up (PIFU) and See on Symptom.** A follow-up outcome with a time-limited window, the conditions under which the patient may ask, a request route through the existing reply links and texts, a person's review at expiry (discharge or extend), and uptake reported apart from discharges. Backed by Welsh Government and Audit Wales; NHS England's guidance (v2, September 2025) sets 5% of outpatient activity on PIFU by March 2029. → NEW-3. Sources: <https://audit.wales/sites/default/files/publications/3750A2024_swansea_bay_follow-up_outpatients_report_final.pdf>, <https://www.england.nhs.uk/long-read/implementing-pifu-guidance-local-health-care-systems/>.
- **Event subscriptions** (FHIR topic-based Subscriptions, native in R5 and through the R4 Backport IG), fed from the outbox that already exists. This is the plumbing for the Wales App, PROMs, video and transport providers. No standard Appointment topic exists, so we define ours. → NEW-4. Source: <https://hl7.org/fhir/uv/subscriptions-backport/STU1/components.html>.
- **Interpreter need and booking.** Record the language, BSL or deafblind need; a booking makes an interpreter request (a staff task first: WITS, the Wales Interpretation and Translation Service, books by portal, phone or email and has no API found) and lengthens the slot; a day-before check lists unconfirmed interpreters. The All-Wales accessible-communication standards now cover people whose main language is neither English nor Welsh. → NEW-5. Source: <https://www.wits.wales/wp-content/uploads/Good-practice-interpreters.pdf>.
- **Reasonable adjustments as a coded record**, shaped like NHS England's Reasonable Adjustment Digital Flag (DAPB4019 v2.0.0) and exposed as a FHIR Flag; in Wales it answers the sensory-loss standards. CAP-4's accessibility needs are the start. → NEW-6. Source: <https://standards.nhs.uk/published-standards/reasonable-adjustment-digital-flag>.
- **Carer and proxy authority.** Today a carer is only a contact. A proxy relationship with scopes (receive, confirm, cancel, rebook), a recorded basis, an expiry, and every action audited as the proxy. The NHS Wales App announced carer proxy access in October 2025 (sources disagree on whether it is live). → NEW-7.
- **Clinic templates and demand and capacity.** Versioned, approved session templates with slot types (new, follow-up, urgent, PIFU) and ring-fencing; a report of required against available capacity and sustainable list size using NHS England's published core and caseload model formulas (which already allow for missed appointments). This, not an overbooking solver, is the capacity answer. → NEW-8. Source: <https://www.england.nhs.uk/ourwork/demand-and-capacity/models/>.
- **Audit export as FHIR AuditEvent** in the IHE Basic Audit Log Patterns, and read auditing ("who viewed this patient"), which the PAS does not record today. → NEW-9. Source: <https://profiles.ihe.net/ITI/BALP/content.html>.
- **A standard scheduling API**: `$find`, `$hold`, `$book` per IHE ITI Scheduling (R4) over the rebook options and booking that exist, so a national app or BaRS can book without custom code. → NEW-10. Source: <https://profiles.ihe.net/ITI/Scheduling/ITI-115.html>.
- **Referral triage outcomes.** WPRS (the Welsh Patient Referral Service) lets consultants prioritise, return with advice or redirect; the PAS should accept "returned with advice" and "redirected" as referral states now, ahead of any integration. → NEW-11.

**Needs an outside party (build the PAS side; the contract or onboarding is the blocker).**

- **NHS Wales App feed.** Since January 2026 the app shows waiting-list status, referrals and appointment notifications (about 80,000 referrals, 197,000 notifications); Welsh Government wants to match the English app "and then go further". No public API documentation was found; the developer portal says "awaiting further development" and the sandbox is moving to the new NHS Wales API Platform. Build the outbound events and the R4 contract (NEW-1, NEW-4); onboarding needs DHCW. → NEW-12. Sources: <https://dhcw.nhs.wales/news/latest-news/people-in-wales-can-now-track-their-nhs-waiting-list-progress-on-the-app/>, <https://developer.nhs.wales/>.
- **PROMs milestones.** Wales has a national PROMs platform (46 pathways) fed from the PAS. Emit listed, seen, operated and follow-up milestones as events; the connection needs DHCW and the Welsh Value in Health Centre. → NEW-13. Source: <https://vbhc.nhs.wales/professionals>.
- **National channels for NOT-3.** Prefer NHS Notify (England: NHS App, SMS, email, letter; PDS contact lookup; suppresses deceased and restricted records; direct-care senders only) and NHS Wales App messaging over separate carrier contracts. → NEW-14, and a note on NOT-3. Volume figures found are vendor case studies.
- **Staff sign-on through national identity** (NHS Wales NADEX-backed federation; CIS2 in England). Nothing public describes a DHCW clinical sign-on service, so this is a question for DHCW, recorded in `spec/16-open-questions.md` when taken up, not a task yet.
- **BaRS receiver and Wayfinder** apply to English or cross-border deployments only; not tasks until someone deploys in England.

**What not to build, and why.**

1. **Overbooking or a schedule optimiser driven by no-show scores.** The evidence is simulation, not NHS field evaluation; it contradicts NFR-11 and loads the cost onto deprived groups. NEW-8 is the capacity answer.
2. **A patient app or portal of our own.** Wales is putting secondary care into the NHS Wales App. Keep the single-use link pages and publish to the national app.
3. **A referral sender or a demographic master.** WPRS and e-RS send referrals; WDS (the Welsh Demographic Service) and PDS hold demographics. Identity stays upstream.
4. **Automatic removal on no reply, or software choosing who goes on PIFU.** Both are clinical-safety decisions, and DTAC-1 has no Clinical Safety Officer yet.
5. **Converting the core to R4.** Add the R4 edge (NEW-1); do not churn the inside.

**Gaps in the research.** No DHCW API catalogue entries, no WDS API documentation, and nothing confirming a WelshPAS replacement (2024 and 2025 FOI answers say none is planned). Nothing found on what the Welsh Language Standards require of apps specifically; the Commissioner's technology guidance is non-statutory.

## 8. Code quality: review 2026-10-21

Three read-only audits (the Rust API and the Loco dashboard; the SvelteKit UI; CI, scripts and spec), with the high-impact claims re-checked by hand before writing them down. IDs are in `tasks.md` (BUG-8, QUAL-1 to QUAL-24).

**Defects, not style.**

- The Loco dashboard's ward and RTT pages map a `patients` table that migration 6 (`m20260601_000006_delegate_identity`) drops. Their smoke tests pass because both lookups return early on an empty list; one occupied bed or one active pathway would give "relation patients does not exist". The same lesson as BUG-1 to BUG-5: tests on an empty database prove little. → BUG-8.
- `Error::Database` text goes to the client (`api/rest/handlers.rs`, `err.to_string()`), and HL7 ACKs carry `e.to_string()` about 50 times. `DbErr` text names constraints and relations, which breaks the AGENTS rule. The chaos test catches it only when hostile input happens to cause a database error. → QUAL-1.
- Six background loops (outbox, tolerance, no-show, backfill, ladder, reminders) are bare `tokio::spawn` with no lock, no graceful shutdown and no supervision; the ladder sends and then records, so two API replicas could send a patient the same text twice. The runbook does not say "run one replica". → QUAL-2.
- The API container builds from `patient-administration-system-with-rust/Cargo.lock`, a crate-level lockfile that still pins sea-orm 1.1, without `--locked`, and its dependency pre-build ends in `|| true`. The image may not be built from what was tested. → QUAL-3.
- The OpenAPI document served at `/swagger-ui` is the v0.44.0 "Phase 2" stub: 0 `#[utoipa::path]` annotations against about 250 routes, while `spec/09-api-surface.md` calls it "the complete, generated contract". The UI hand-writes its types and its mock, so the Rust API, the TypeScript and the mock can drift with no test failing. → QUAL-4, QUAL-15.
- The spec says FR-39, FR-41, FR-42 and NFR-15 are "Not yet built" though CAP-12, CAP-14, CAP-15 and NOT-5 landed; `generate-status.sh` counts only the bold "Not yet" form, so NFR-17 ("Not built:") is missed. → QUAL-5.

**Structure and duplication (Rust).** 52 `PAS_*` variables are read in 19 files, some on the request path, booleans accept only lowercase `true`, and unparseable values fall back silently (QUAL-6). Raw SQL is stringly typed: about 240 `try_get("", "col")` calls against 6 `FromQueryResult` uses (QUAL-7). About 60 `let _ =` and 130 `.ok()` drop errors without a log line (QUAL-8). Handler boilerplate repeats: 17 identical `audit_ctx`, 31 local response helpers, 36 hand-written transaction-error mappings (QUAL-9). `handlers_hl7v2.rs` is 1,903 lines with 45 repeated early-return arms; `noshow`, `tolerance` and `adt` are each over 1,200 (QUAL-10). "A named person is required" is checked in at least 11 places, only some checking length (QUAL-11). No `[workspace.lints]`; 22 `unwrap()` and 9 `expect()` in non-test code (QUAL-12). 76 "Phase 2/3" comments, dead stub functions, and the stub upstream clients ship in the library (QUAL-13). List endpoints clamp silently at 500 or 1000 with no cursor or truncation flag (QUAL-14).

**Observability.** No `#[tracing::instrument]`, no request id, no metrics: an operator cannot see outbox lag, dead letters or when each job last succeeded (QUAL-16).

**Testing.** No property tests, fuzzing, coverage or mutation testing anywhere. The HL7 parser, the inbound keyword parser, the NHS number check and the Redis protocol parser in the UI are the natural fuzz and property targets; the support-only, consent and k-anonymity rules are where a surviving mutant would matter most. Test fakes are duplicated (11 `MessageSender` fakes). The UI has no unit tests for its server code (`oidc.ts`, `oidc-session.ts`, `resp.ts`, `session-store.ts`, `guard.ts`, `pas.ts`, every `+page.server.ts`), and FR-10's pages are tested only against a mock. The restore drill in CI compares counts on an empty database ("0 = 0"). → QUAL-17 to QUAL-20, QUAL-23.

**UI structure.** Small and modern (3,666 lines, no Svelte 4 syntax, largest component 162 lines). The duplication is in the server code: the `PasError`-to-502 catch appears 10 times, `actingAs(...)` 14 times, and the two token-link routes each define their own helpers. Guards are per route and nothing fails when a new route forgets one. Prettier is defined but not run in CI; no ESLint. → QUAL-19, QUAL-21.

**CI and supply chain.** 40 action references pinned by tag, not SHA, including a third-party release action holding `contents: write`; an expression interpolated straight into a `run:` line in `release.yml`; no actionlint or zizmor; no npm vulnerability gate or CodeQL; development containers listen on all interfaces with default credentials; compose images are not tracked by Dependabot; tool installs compile from source on every run. → QUAL-22, QUAL-24.

**Not a problem.** Duplicate crate versions (57) are almost all upstream (`rusty_paseto`, `tera`); little to gain locally. The UI's component size and Svelte syntax are fine.

## 9. Suggested order for sections 7 and 8

1. **Defects first:** BUG-8, QUAL-1, QUAL-2, QUAL-3, QUAL-5. Each is small and each is a real fault or a false statement.
2. **The contract:** QUAL-4 then QUAL-15. A real OpenAPI document is what NEW-1, NEW-10 and the UI's types all stand on.
3. **Guardrails that stop regressions:** QUAL-12, QUAL-17, QUAL-19, QUAL-21, QUAL-22, QUAL-23.
4. **Capabilities buildable now, in value order:** NEW-2 (validation campaigns, reusing CAP-8), NEW-3 (PIFU), NEW-1 (R4 edge), NEW-4 (subscriptions), NEW-5, NEW-6, NEW-7, NEW-8, then NEW-9, NEW-10, NEW-11.
5. **Refactors as their modules are touched**, not as a separate project: QUAL-6 to QUAL-11, QUAL-13, QUAL-14, QUAL-16, QUAL-18, QUAL-20, QUAL-24.
6. **Waiting on others:** NEW-12, NEW-13, NEW-14 (DHCW, the Value in Health Centre, NHS Notify onboarding). Start the conversations early; they have the longest lead time.

## 10. Second pass: research 2026-10-21 (evening)

A second, independent research pass and code audit, run after sections 7 to 9 were written. Only what those sections do not already cover is recorded here; the rest confirmed them. IDs continue in `tasks.md` (NEW-15 to NEW-21, QUAL-25 to QUAL-33). Evidence marks: **[P]** primary (government, NHS, HL7), **[S]** secondary (audit, FOI, news).

**Capabilities.**

- **Follow-up target dates and the Welsh delayed-follow-up return.** Health boards report monthly how many follow-up patients are past their target date, booked and not booked, banded by how far past (DHCW change notice DSCN 2020/18 [P]). Audit Wales counted 29,129 delayed follow-ups at one board in October 2025 [S]. The PAS has no target date on a follow-up or a waiting-list entry (0 hits for `target_date`). This is the measurement PIFU (NEW-3) is judged by. → NEW-15. Sources: <https://dhcw.nhs.wales/data/information-standards/data-standards/data-standards-files/data-standard-change-notices-docs/dscns-2020/20200930-dscn-2020-18-outpatient-follow-up-delay-reporting-v1-0-pdf/>, <https://audit.wales/sites/default/files/publications/3750A2024_swansea_bay_follow-up_outpatients_report_final.pdf>.
- **Welsh RTT rules.** `models/rtt.rs` has `Started`, `Paused`, `Resumed`, `Stopped` and one breach threshold, which is the English shape. In Wales the clock starts when the hospital receives the referral, and a could-not-attend resets the clock rather than adjusting it; an Ombudsman finding concerns a wrongful reset. The newest rules document found is version 7 (2017), seen only through an FOI disclosure, so the current text must come from Welsh Government or DHCW before this is built. → NEW-16. Sources [S]: <https://cavuhb.nhs.wales/about-us/governance-and-assurance/freedom-of-information/disclosure-log/2020/months/december-2020/foi-20-465-rules-for-managing-referral-to-treatment-pdf/>, <https://www.ombudsman.wales/findings/referral-to-treatment-time-swansea-bay-university-health-board-4/>.
- **Clinical priority codes with review-by dates.** Surgical waiting lists carry a clinician-set priority (P1 to P4, plus a patient-choice category whose number sources disagree on) and a date by which it must be reviewed. `waitlist::Priority` has `Routine`, `Urgent`, `TwoWeekWait`, `Emergency` only. The code must be set by a clinician and must not reorder offers by itself (NFR-11 still holds; clinical ordering is a person's). → NEW-17. Sources: <https://www.england.nhs.uk/coronavirus/secondary-care/other-resources/clinical-prioritisation-programme/frequently-asked-questions-about-surgical-prioritisation/> [P], <https://www.boa.ac.uk/asset/CBE0C0ED-40C3-405F-A5DAFFE7F8C2F350> [S].
- **Group consultations.** Welsh Government rolled out virtual group clinics across NHS Wales [P]. The PAS enforces one live appointment per slot (FR-155). A declared capacity on group sessions only, with an outcome per attendee and the one-to-one option always offered, is not overbooking. → NEW-18. Source: <https://www.gov.wales/virtual-group-clinics-be-rolled-out-across-nhs-wales>.
- **Bulk export.** FHIR Bulk Data Access 2.0 (`$export`, asynchronous, NDJSON) under SMART Backend Services is how a national data platform pulls activity without per-record calls [P]. It must follow CAP-11's privacy rules for any use that is not direct care, and needs a consumer's agreement. → NEW-19. Source: <https://hl7.org/fhir/uv/bulkdata/STU2/authorization.html>.
- **Accessible Information Standard 2025: the review step.** The revised standard (DAPB1605, July 2025) adds "review" to identify, record, flag, share and meet [P]. CAP-4 records needs but not when they were last reviewed. Small, and it pairs with NEW-6. → NEW-20. Source: <https://www.england.nhs.uk/long-read/accessible-information-standard-requirements-dapb1605/>.
- **The FHIR surface says more than it does.** There is no `/fhir/metadata` (no `CapabilityStatement` anywhere) and `POST /fhir/Appointment` returns the Phase 2 `upstream_stub` 503 (`api/fhir/handlers.rs:339`). A client cannot discover what works. → NEW-21, a prerequisite for NEW-1.

**Code quality, beyond section 8.**

- **No clock abstraction.** 159 `Utc::now()` in `src` and 93 SQL `now()`. `AGENTS.md` lists three tests that failed at certain times of day and were fixed by working around the clock, not by controlling it. → QUAL-25.
- **Four down migrations do nothing** (`m20260615_000020_audit_reasons`, `m20260621_000026_attendance`, `m20260622_000027_referral_management`, `m20260624_000029_letter_only`), so up, down, up fails. → QUAL-26.
- **Serialization failures become a 200 with `null`.** 163 `serde_json::to_value(..).unwrap_or(Value::Null)` in the server, and `HandlerResult` is a tuple rather than a `Result`. QUAL-9 removes repeated helpers; this is the correctness half. → QUAL-27.
- **String-typed states.** At least 20 `pub status: String` or `pub kind: String` fields, while the database holds 111 CHECK constraints for the same values. A Rust enum per state makes a typo a compile error and keeps the CHECK and the code in step. → QUAL-28.
- **Token helpers copied.** `new_token()` and `hash()` are identical in `backfill/mod.rs:50` and `reply_links/mod.rs:114`; a security primitive should exist once. → QUAL-29.
- **`routes::router` is one 739-line function** with no `.nest` or `.merge`, and `main` is 438 lines. QUAL-10 targets files; this targets the two functions everyone edits. → QUAL-30.
- **Three operator interfaces.** The Loco dashboard (2,633 lines), the server's own Tera dashboard (`api/dashboard.rs`, 425 lines) and the SvelteKit UI overlap. The Loco crate is why the workspace carries `tera` 1 and 2 and `tower-http` 0.6 and 0.7, and it triggered DEP-2's number workaround; BUG-8 shows it has rotted. Keeping it is a person's decision. → QUAL-31.
- **NFR-1 and NFR-2 are untested** as general properties (spec 07: "Not yet tested"): an audit row and an outbox row in the same transaction as every change. → QUAL-32.
- **Five operator switches are undocumented** anywhere outside the code: `PAS_CLINIC_ADDRESS`, `PAS_DEFAULT_COUNTRY_CODE`, `PAS_REBOOK_HORIZON_DAYS`, `PAS_REBOOK_MIN_NOTICE_HOURS`, `PAS_REBOOK_OFFER_MINUTES`. QUAL-6 fixes the cause; the runbook can be fixed today. → QUAL-33.

**Not proposed, and why.** LLM-drafted referral summaries or letters: NHS England's guidance treats generative summarisation as a likely medical device (MHRA <https://www.gov.uk/government/organisations/medicines-and-healthcare-products-regulatory-agency>, a Clinical Safety Officer, DCB0160), and it is outside the "administrative, not clinical" scope; revisit only after DTAC-1. CloudEvents headers on webhooks: superseded by NEW-4. Multi-tenancy across health boards: no evidence found that Wales wants it.

**Where these fit in section 9's order.** QUAL-26, QUAL-29, QUAL-32 and QUAL-33 are small and go with step 1. QUAL-27 goes with QUAL-4 in step 2 (both change handler signatures). QUAL-25 goes with step 3. NEW-21 goes before NEW-1; NEW-15 goes with NEW-3; NEW-17, NEW-18 and NEW-20 follow NEW-8 and NEW-6; NEW-16 waits for the rules text; NEW-19 waits for a consumer. QUAL-31 needs a decision from the owner.

## 11. Manchester Triage System: research 2026-10-23

The Manchester Triage System (MTS) is the most widely used emergency triage method in the UK, Europe and Australia. A nurse picks a presentational flowchart for the presenting complaint (sources count 52 to 64 charts) and works down ranked discriminators, general or specific, until one fits. The result is one of five categories with a target time to first clinician contact: Immediate, red, 0 minutes; Very urgent, orange, 10; Urgent, yellow, 60; Standard, green, 120; Non-urgent, blue, 240. One PDF of unknown origin gives yellow as 30 and green as 60; treat that as an outlier and make targets configuration. The current text is *Emergency Triage*, 3rd edition, version 3.8 (2023; a listing mentions a 4th-edition preface, so check the edition with the publisher). Sources: <https://www.alsg.org/en/files/Introducing%20Manchester%20Triage%20System.pdf>, <https://www.wiley.com/en-gb/Emergency+Triage:+Manchester+Triage+Group,+3rd+Edition-p-9781118299067>, <https://metricgate.com/docs/manchester-triage-priority/>.

**What this means for the PAS.** Scope (`spec/02-scope.md`) puts ADT in and clinical records out, and the PAS has no emergency-attendance concept that was found in this pass (check ADT before NEW-22). So the PAS should *record* a category that someone else decided, and measure waits against it. It should not compute a category.

- **Licence.** The flowcharts and discriminators are licensed IP. ALSG's guidance has separate licences for software companies (framework only, or framework plus content), single-organisation and in-house builds; Wiley's digital licensing team handles electronic use. Source: <https://alsg.zendesk.com/hc/en-gb/articles/4405111382930--MTS-Licencing-Manchester-Triage-System-MTS-content>. An open-source repository must not carry the content. → NEW-25.
- **Clinical safety.** Triage is clinical decision support; DTAC-1 has no Clinical Safety Officer yet. → NEW-25.
- **Buildable now.** Record the category with system, version, assessor and time (NEW-22); measure time to first assessment against configurable targets, reported apart from any queue order (NEW-23); map to FHIR and HL7 v2 (NEW-24).
- **Evidence gaps.** No official FHIR code system for MTS categories was found; the only SNOMED CT reference is a community forum example (713011005, "Assessment using Manchester Triage System"), unverified. No NHS Wales or NHS England emergency data set field for the triage category was confirmed in this pass. The flowchart contents were not read, deliberately. Source for the SNOMED example: <https://community.fhir.org/t/manchester-triage-system-as-riskassessment/1743>.

**Not proposed.** A triage calculator or flowchart engine (licence, clinical safety, medical-device status); automatic re-ordering of a queue by category with no person (a clinical decision, same reasoning as NEW-17); an ED tracking board for clinicians (clinical record territory, other systems own it).

**Order.** NEW-25 first (it is a person and a decision, and it gates the rest), then NEW-22, NEW-23 (after QUAL-25 for the clock), NEW-24.

## 12. Wong-Baker FACES Pain Rating Scale: research 2026-10-23

The Wong-Baker FACES® Pain Rating Scale is a self-report tool: six cartoon faces, scored 0, 2, 4, 6, 8, 10 (no hurt to hurts worst), used from about age 3 (the publisher; the scale was made for children). Validation evidence found is for children: a paediatric emergency department study of 120 children aged 8 to 17 found strong agreement with a visual analogue scale (rho 0.90); nothing was found on children under 8 or on adults. The foundation says it must be self-report, the patient must understand the instructions and pick a face, it must not be used with unresponsive patients, and it is for pain only (not mood or comfort). Sources: <https://wongbakerfaces.org/?p=4>, <https://wongbakerfaces.org/licensing-dashboard/licensing-web-form/>, <https://researchconnect.stonybrook.edu/en/publications/validation-of-the-wong-baker-faces-pain-rating-scale-in-pediatric/>, <https://rdcoas.c-path.org/orpheus_resource/wong-baker-faces-pain-rating-scale/>.

**What this means for the PAS.** A pain score is a clinical observation, and the scope (`spec/02-scope.md`) keeps clinical records out. So the PAS should not host the scale, show the faces or collect a score as a feature. Little here is buildable, and that is the finding.

- **Licence and artwork.** The faces are copyrighted. The foundation licenses healthcare organisations by a web form with a quote (no fees published); it forbids modifying the faces or adding colour. A search snippet said an organisation licence covers electronic medical records, but the licensing page itself says nothing about software, so confirm in writing. The repository must not contain the face images (same rule as NEW-25 for Manchester Triage). The foundation's translations are separate artwork-and-text files; the project's own translations (CLAUDE.md) must not include the scale's wording.
- **Interchange only, if a consumer wants it.** If a clinical system sends a pain score to a PAS-owned view (for example a discharge or ward-board summary), carry it as an opaque coded observation. Candidate code: LOINC 38221-8, "Pain severity - Wong-Baker FACES pain rating scale", beside the generic 38208-5 "Pain severity - Reported" in the LOINC Pain Assessment panel (38212-7). Taken from search results about value sets; the LOINC page itself returned 403, so the code and its answer list are **unverified**. No consumer was found, so no task builds this.
- **Communication needs (CAP-4, NEW-6, NEW-20).** The useful link: a patient who cannot use a numeric scale (a child, a person with a learning disability, limited English, aphasia) benefits from a pictorial or alternative pain tool, and the Accessible Information Standard asks services to record and flag format needs. Check whether the needs vocabulary has a value for "prefers a pictorial or simplified pain scale" (and, with NEW-6, whether it belongs among reasonable adjustments). That records a preference; it does not hold a score or the faces.
- **Not for non-responsive patients.** Any future display of a score must carry which tool and which version produced it, because FACES, a 0 to 10 numeric scale, FLACC (observed, non-verbal) and others are not interchangeable, and a bare number invites misreading.

**Not proposed.** A pain-scale widget or score capture in the SvelteKit UI (clinical record territory, licence, clinical safety under DTAC-1); embedding the images or any recolouring; a pain trend chart; using FACES for patient satisfaction or mood on reminders or portal pages (the licence forbids use for emotion, mood or comfort, and it would tempt a "smiley" survey; any experience survey needs a different instrument).

**Order.** NEW-26 (a vocabulary check, small) when NEW-6 is next touched; NEW-27 is a written decision, not code, and only if a consumer asks.

## 13. Hazard closure: research and assessment 2026-10-28

The request: only the Clinical Safety Officer can remove a hazard, and must put in evidence that it has been removed. The rule, reworded, is in [`spec/hazard/index.md`](spec/hazard/index.md). The assessment:

- **Right in substance.** Secondary summaries of DCB0129 say the CSO approves hazard assessments and risk acceptability, that each hazard is re-scored after controls, and that entries trace to the requirement, design decision, test or code change behind each control. Evidence-before-closure is the same idea. Sources (commentary, not the standard; check against it): <https://presencis.com/regulations/dcb0129/article-hazard-log/>, <https://presencis.com/regulations/dcb0129/article-risk-evaluation/>, and the standard itself, <https://digital.nhs.uk/data-and-information/information-standards/governance/latest-activity/standards-and-collections/dcb0129-clinical-risk-management-its-application-in-the-manufacture-of-health-it-systems>.
- **Wording changed.** The role is the *Clinical* Safety Officer; the rule says *close*, not *remove*, because a log that loses rows cannot show what was known at the time of an incident.
- **"Removed" has four meanings** (eliminated, controlled, accepted, not applicable), each needing different evidence. Treating them as one is how a hazard gets "removed" while the risk stays.
- **A single approver is a risk, not only a control.** The CSO is the only person who may close, but should not be the only person who checks. The repository already uses proposer-differs-from-approver for templates, no-show reviews and validation decisions; the same pattern applies. A CSO who wrote the control gets a second clinician.
- **Evidence has to outlast the link.** Commit hash, test name with CI run, content hash for documents; never edited, only corrected by a later entry.
- **Cumulative risk.** Many acceptable risks can add up to an unacceptable whole; the case report must consider the total.
- **No CSO today, so nothing can close.** That is the right failure. SAFE-1 is a person and a decision, and it gates the rest.
- **Where the register should live is open.** In git it has history and review for free. In the database (SAFE-4) it needs a CSO role claim, append-only events and its own tests, and it would put a safety document inside the system it describes. Recommendation: do SAFE-1 to SAFE-3 and SAFE-5 first and decide on SAFE-4 only if a CSO asks for it.

**Not proposed.** A route that lets any administrator close a hazard "on the CSO's behalf"; a closure with a free-text "see ticket" as the only evidence; automatic closure when a linked test passes (a passing test is evidence, not a clinical judgement); scoring likelihood and severity in code (a clinician's job).

**Order.** SAFE-1 (a person), then SAFE-2 and SAFE-3, then SAFE-5 and SAFE-6, and SAFE-4 only on request.

## 14. Openfront Hospital: research and advice 2026-11-02

Source: <https://github.com/openshiporg/openfront-hospital> (read from the public repository; nothing was run). Full comparison: [`spec/compare/openfront-hospital/index.md`](spec/compare/openfront-hospital/index.md). Tasks: `OFH-1` to `OFH-9`.

- **What it is.** A whole-hospital app (63 lists: storefront, scheduling, encounters, inpatient, orders, labs, claims, inventory) in Next.js, KeystoneJS and Prisma, MIT, one star and 30 commits, built from a starter shared by about 30 sibling "Openfront" apps. Early, fast-moving, thinly tested in the repository (three test files visible), US-shaped, no FHIR or HL7.
- **Not a fork or a dependency.** Other stack, clinical and billing scope that we exclude, no standards interchange.
- **Worth borrowing as ideas.** Optimistic concurrency with an operation key on every controlled write; auditing that a *search* resolved personal data, without the values; legal hold and recorded disposition; timed bed reservation with an occupancy log; an arrival queue; per-area capability flags; a once-only bootstrap; a schema map page.
- **Where we lead.** Standards, safety case, DTAC, fairness measurement, languages and accessibility, CI that cannot silently skip, delegated identity.
- **Risk.** Its anonymous booking creates patients from visitors; weigh before copying (OFH-8).
- **Order.** OFH-1 and OFH-2 first (a real lost-update hazard and a gap we already admit); then OFH-3, OFH-4, OFH-7; the rest wait for a person to ask (OFH-5, OFH-6) or a decision (OFH-8, OFH-9).
- **Limit.** Our "not found" findings are text searches of `src/` and `migrations/`; each OFH task begins with a proper check.

## 15. Care Coordination and 360 Referrals: research and advice 2026-11-02

Source: <https://kevinmayfield.github.io/PatientCareCoordination/en/index.html> (five pages read; the rest not). Full comparison: [`spec/compare/patient-care-coordination/index.md`](spec/compare/patient-care-coordination/index.md). Tasks: `PCC-1` to `PCC-8`.

- **What it is.** An FHIR R4 implementation guide, v0.1.0, one author, a draft build dated today, that restates IHE 360X (the closed loop referral) for NHS England. It says it is a proof of concept, not authoritative, not endorsed, and not for implementation. England only; nothing Welsh.
- **Not a standard to build to.** Take the *rules* and check each against IHE 360X and the real NHS and Welsh sources before building.
- **The finding that matters.** The PAS is the recipient-side system. It receives, triages, waits, books and closes referrals, but **nothing tells the referrer**. A referral that is booked, returned with advice, redirected, closed or missed is known only inside the PAS. The loop is open at our end.
- **Cheap and real.** The HL7 v2 publisher already sends SIU S12, S13, S14, S15 and S26, and `appointments.referral_id` exists, but SCH-26 (the referral ID) is left empty. Filling it is one small change (PCC-1).
- **Worth taking as rules.** Decline can follow acceptance; a cancel can lose to an outcome that arrived first; a no-show leaves the referral open; one sender-assigned referral ID in every message; label every identifier's type and assigner; no planned encounters (we create one in `pre_admit`: PCC-8); keep an external standard behind an anti-corruption layer and keep the raw bundle.
- **Not taken.** The clinical halves (consultation note, outcome document, discharge report, care plans): out of scope. Extra states (`information_requested`, `deferred`, `cancel_requested`) until a referral-handling service asks (PCC-3). Plan-pointer flags until a consumer appears (PCC-6).
- **Risks.** Building to a draft; cancelling a referral already acted on; a new outbound message needs a named recipient and agreement; England-shaped codes (ODS, UBRN) must stay out of the core (QUAL-40).
- **Order.** PCC-1 and PCC-2 first; PCC-5 and PCC-8 are small; PCC-4 waits for a recipient; PCC-3, PCC-6 and PCC-7 wait for a person or a consumer.
- **Limit.** "Not found" statements are text searches of `src/` and `migrations/`.

## 16. Local CI/CD: research and advice 2026-11-04

Asked: do more with local CI/CD rather than rely on GitHub, which is unreliable and costs money. Method: this repository's own CI history (`gh run list`, 100 runs from 2026-10-21 to 2026-11-04), the workflow files, this machine, and web sources read today. Nothing was installed, registered or pushed.

**What the evidence says, including where it does not match the question.**

- **Money: not for this repository.** It is public. GitHub's own page says "GitHub Actions will remain free for public repositories", standard GitHub-hosted or self-hosted. The $0.002 per minute charge on self-hosted runners (planned for 2026-03-01) was postponed: "to take time to re-evaluate our approach"; no new date is given. Hosted prices fell by up to 39% on 2026-01-01. Money only comes in if the repository goes private, if larger runners are used, or if the postponed charge returns. If you pay for something else on GitHub, tell me what, because the answer changes. ([GitHub, 2026 pricing changes](https://github.com/resources/insights/2026-pricing-changes-for-github-actions); [changelog](https://github.blog/changelog/2025-12-16-coming-soon-simpler-pricing-and-a-better-experience-for-github-actions/))
- **Reliability: yes, and it is the better reason.** Third-party trackers (not GitHub's status page) list Actions as GitHub's most incident-prone service: 14 outages and about 29 hours between 2026-03-03 and the summer; a roughly 10-hour event on 2026-08-06; three incidents on 2026-10-01, the longest over 9 hours. Treat the counts as indicative. ([Pingoru](https://pingoru.io/providers/github-actions/outage-history), [IncidentHub](https://blog.incidenthub.cloud/github-actions-pages-outage-aug-6-2026), [UptimeRobot](https://uptimerobot.com/is-it-down/github/2026-10-01/)) Our own record: of 100 CI runs in two weeks, 86 passed, 8 failed and 6 were cancelled; mean 10.6 minutes, longest 19. **I have classified only one failure as the platform's fault**: the 2026-11-03 run of `a740395`, where the runner ran out of disk (`No space left on device`) while linking; the same commit's re-push passed. I have not read the other seven.
- **The local gate is not faster today.** My gates this week took 12 to 25 minutes against CI's 10.6, because (a) another session shares the working tree and the `target/` lock, (b) a cold clippy build costs 12 to 15 minutes, and (c) one run was broken by a **Postgres container clock 74 minutes behind the host** (podman VM drift after sleep), which failed two time-based tests that pass on CI. So local CI gains come from isolation and caching, not raw speed. This Mac has 16 cores and 128 GB, more than a 4-core hosted runner, but I have not measured it.
- **The two machines differ.** Local is arm64 macOS; CI is x86_64 Linux. A local pass does not prove the Linux image or an x86_64-only bug. Some check on Linux x86_64 stays useful.

**Assessment of the options.**

| Option | What it gives | Cost and risk | Verdict |
|---|---|---|---|
| **One entry point per job (`just`)**, workflows call it | The same command runs on a laptop, GitHub, Forgejo, GitLab. Removes the main lock-in (11 jobs of YAML nobody can run locally). `just` is already installed. | A day. Low. | **Do first.** |
| **Local gate in a clean worktree** with its own `target/`, a fresh database, a clock check, a signed-off result file | Fixes the contention and the drift we hit; gives a reproducible record per commit. | Two days. The result is only as trusted as the machine. | **Do second.** |
| **Post the local result as a commit status** (`ci/local`) | Branch protection can see it; GitHub stays the mirror. | Needs a token (outward action: ask first). One person's machine becomes the authority. | A decision (QUAL-43). |
| **Pre-push hook, quick subset** | Catches format, generated-file, link and lint failures in about a minute. | Slow hooks get bypassed; keep it under two minutes. | Do, opt-in. |
| **Self-hosted runner on this Mac** (GitHub runner, Forgejo runner, Woodpecker agent) | Same workflows, local hardware. | **A public repository must not run fork pull requests on a self-hosted runner** (GitHub's documented guidance; not re-checked today). Woodpecker on macOS supports only its local backend: no isolation, no services ([Woodpecker docs](https://woodpecker-ci.org/docs/administration/installation/supported-platforms)). Forgejo's macOS and host-mode support is not stated in the pages I could read. | **Not recommended** here. If used, push events on `main` only. |
| **Second independent CI** (Forgejo Actions or Woodpecker on Codeberg; GitLab CI) | An outage on one is not an outage of the gate. The remotes already exist: origin pushes to GitHub, Codeberg and GitLab. | Codeberg's hosted Woodpecker needs an access request, is "as-is and might break at any time", linux/amd64 only, "reasonable" use ([Codeberg docs](https://docs.codeberg.org/ci/)). GitLab's free allowance is reported as 400 minutes a month, or 50,000 under its open source programme; I could not confirm either today. Own Linux host = the cost you want to avoid. | A decision (QUAL-44). Cheapest honest step: ask Codeberg for access and run the same `just` recipes. |
| **Dagger** (pipelines as code, same locally and in CI) | Container steps (restore drill, image builds, FHIR validator, Keycloak) cached and portable. Installed here. Docs mention Podman. | The official SDK list shows Go, PHP, Python and TypeScript; Rust is a separate community crate. Another language and engine to learn. | **Later, only if** the `just` recipes grow tangled. |
| **`act`** (run GitHub workflows locally) | Quick smoke of a workflow file. | Weak on service containers, caches, third-party actions and tokens; our workflows use all of them. | Not as the gate. |

**Measured 2026-11-07, on commit `bc3704a`.** Local is arm64 macOS on a busy machine, with a warm build directory. CI is x86_64 Linux, cold, and its database job runs under coverage, so this is not a like-for-like race.

| Job | Local (seconds) | CI (seconds) |
|---|---:|---:|
| `rust` | 98 | 373 |
| `rust-db` | 64 | 395 |
| `deny` | 2 | 20 |
| `ui` | 14 | 55 |
| `docs` | 1 | 8 |
| **The five together** | **179** | **851** |

CI's other jobs have no local equivalent in the gate: container images 622 and 41, the FHIR validator 517, browser suites 278, fresh clone 301, restore drill 62, workflows 10. CI took 10.5 minutes from end to end. So a warm local gate answers in 3 minutes and covers the five jobs only.

`cargo nextest` against `cargo test`, same tree, same machine. These are small samples: two runs each for unit tests, one for database tests, under load. Unit tests: 12 against 25 seconds, but 688 tests against 693, because `nextest` does not run doc tests. Database tests, serial, on a fresh database: 95 against 68 seconds, with 445 tests on both, because `nextest` starts a process for each test. So the database tests stay on `cargo test`, and the unit saving does not pay for another tool in CI. The time worth winning is the image build.

**Advice, in order.** (1) QUAL-41 `just` recipes and thin workflows. (2) QUAL-42 the clean local gate with the clock check, measured against CI's times. (3) QUAL-45 the quick pre-push hook. (4) QUAL-46 `cargo nextest` and a build cache, only if the measurements in QUAL-42 show where the time goes. (5) Then decide QUAL-43 (status) and QUAL-44 (second CI) with real numbers. **Keep GitHub as the independent x86_64 check** until a second one exists; do not make a laptop the only gate for software that feeds clinical systems.

**Do not.** Switch the repository private to "save" money that is not being spent. Run fork pull requests on a self-hosted runner. Make the local result a required check without a second reader.

**Limits.** Outage counts are from aggregators. GitLab's current free minutes, Forgejo on macOS, Dagger's Podman requirements and the self-hosted-runner security guidance were not read from primary pages today. The cost of the Mac as a CI host (power, sleep, being used for other work) is not counted.

## 17. SeaORM to sqlx: staged plan 2026-11-06

Asked for again 2026-11-06 (SQLX-1). Measured from the tree: 253 files use `sea_orm` (244 in the server and its migrations, 9 in the front-end); 486 raw-SQL calls and 470 `Statement` builders (src and tests); 66 transaction closures; 29 entities; 426 lines of `ActiveModel` use; 57 migrations. Nothing below is built. axa-sqlx 0.1.2 gives pooling, migrations and test resets, and on purpose does not wrap queries. axa 0.5, sea-orm 2.0 and axa-sqlx all sit on sqlx 0.9.

**Status: done 2026-11-06. `sea-orm` and `sea-orm-migration` are out of `Cargo.lock`.**

**What sets the order.**

- **The front-end does not depend on the server.** This plan first said it could not go before the migrations. That was wrong: `SqlxPostgresConnector::from_sqlx_postgres_pool` lets sea-orm's `Migrator` run on an sqlx pool. After stage 1 even that is not needed: `migration::up` takes a `PgPool`. The front-end's stage can land at any time.
- **Repositories cannot move one at a time.** A sea-orm `DatabaseTransaction` cannot be handed to sqlx, and the audit and outbox repositories run inside every transaction closure. Converting them piecemeal does not compile. So a project-owned connection layer goes in first, on top of sea-orm, and the implementation behind it is swapped in one place.

**Stages. Each one ends with the whole gate green and is pushed on its own.**

| Stage | What | Risk to watch |
|---|---|---|
| 0 | Land the work already finished. Done. | |
| 1 | Migrations become `.up.sql` and `.down.sql` files under `sqlx::migrate`. The 57 keep their order. A database that has `seaql_migrations` is adopted into `_sqlx_migrations`, not re-run. `pas-migrate` and the front-end's `db migrate` follow. | History lost or re-applied. Migrations that build SQL in Rust (four use the schema manager) must give the same schema: compare `pg_dump --schema-only` of old and new, fresh and adopted. `tests/qual26_migrations_test.rs` keeps up and down. |
| 2 | `crate::db` gains a small connection trait (execute, fetch one, fetch optional, fetch all, with bound values) and an `in_transaction` helper shaped like today's closure, both implemented over sea-orm. Every `Statement` site and all 66 closures move to it. | Bind types: `timestamptz`, `uuid`, `jsonb`, and arrays. |
| 3 | The 29 entities and every `ActiveModel` become plain SQL and row types in the repositories, still on sea-orm's connection. | A partial update (`NotSet`) must stay partial. `updated_at` is the version behind `ETag` (NFR-27): it must change exactly when it does now, at the same precision. `FixedOffset` to `Utc` must not move an instant. `FOR UPDATE` must stay on every locked read (H-15). |
| 4 | Swap the layer to `sqlx::PgPool` and `sqlx::Transaction`. `Error::Database` carries `sqlx::Error` (687 sites) and unique-violation handling moves to `axa_sqlx::is_unique_violation`. | Commit and rollback on a closure's error, and on a panic. Pool settings stay as they are. |
| 5 | The front-end moves: axa's `sqlx` feature, its models, `axa::Db` as `PgPool`. | Its read models must match the columns the server's stage 1 left in place. |
| 6 | `sea-orm` and `sea-orm-migration` out of `Cargo.lock`; docs, `AGENTS`, the schema map, `cargo deny`. Restore drill, chaos tests, FHIR validator, both browser suites. | |

**Queries are `sqlx::query`, checked at run time, by default.** They keep today's SQL strings, so the diff shows what changed. `sqlx::query!` would check them at compile time, but it needs a database or a committed `.sqlx` cache at every build, including the image build. Moving to it is a separate, later choice.

**Stage 1, how it was done and checked (2026-11-06).** The SQL was not retyped. The old migrator ran up and then down on a throwaway PostgreSQL 18 with `log_statement=all`, and each file was written from what the server received, so SQL that Rust assembled is included: 57 up, 51 down, no bound parameters, no empty bodies. Comments at the top of each old file were kept; comments inside them are in git history. `pg_dump --schema-only` of a fresh database from the SQL files equals SeaORM's, apart from the `pg_dump` version line. A copy of the long-used development database was adopted: 57 rows, no migration executed, schema unchanged, every checksum accepted. `tests/sqlx1_migration_history_test.rs` covers a finished database, one part-way through, one newer than the binary, a gap in the record, a revert after adoption, and an empty database.

**Stage 2, how it was done (2026-11-06).** `db::sql` holds `Stmt`, `Param`, `Row`, `Conn` and `Transact`, still on sea-orm. All 467 statements, 459 raw calls and 66 transaction closures use them, and `Conn` takes `&self` for a pool and a transaction alike, so at the swap a transaction can sit behind a lock and no `&txn` call site changes. `is_unique_violation` and `is_rejected_statement` hold the two places that read a database error. `tests/sqlx1_connection_layer_test.rs` pins commit, rollback, bound types, microsecond timestamps, error classes and row locks; it is what stage 4 must leave passing. 

**Stage 3, how it was done (2026-11-06).** The 112 builder queries are literal SQL in the repositories, so a reviewer reads what runs and the prepare test can check it. They were generated from the old chains and reported where they could not be parsed; six were written by hand. Each entity is a plain `Model` with `FromRow` and `Table`. `Patch` writes only the columns it names, which is what `ActiveModel.update` did and what stops a lost update; `update_row` is the whole-row form and never rewrites `created_at`. `insert` returns the stored row, which `release_class` depends on. All 11 builder locks became `FOR UPDATE`, checked per file: 40 before and after. sea-orm is left in `db/sql.rs`, `db/row.rs`, `db/mod.rs` and `error.rs`.

**Stage 4, how it was done (2026-11-06).** `Pool` is `sqlx::PgPool`. `Txn` holds a `sqlx::Transaction` behind a lock, so no `&txn` call site changed, and a transaction dropped without a commit rolls back, panics included. `Param` is an enum of the column types the system binds. The pool keeps its settings: 10 connections at most, 2 at least, a 5-second acquire timeout and 60 seconds idle. sea-orm's `connect_timeout` of 8 seconds never applied, because it sets the same option as `acquire_timeout` and is overwritten by it. `tests/sqlx1_connection_layer_test.rs` passed with no change to it. `is_rejected_statement` is now narrower than before: a lost connection is no longer read as a refused statement, so `age_band`'s validation message can no longer hide an outage. 

**Stages 5 and 6 (2026-11-06).** Another session moved the front-end, and landed here on the maintainer's word: nine model files on `sqlx::query_as` with bound values, one-to-one with the old queries, and a new test that calls all 13 reads with seeded rows, because the dashboard swallows read errors. The outage drill (`scripts/chaos-podman.sh`, on a throwaway database) passed on the sqlx server: no hung request while the database was stopped, and recovery in one second with no restart. **Limit:** before landing, one run of the front-end's tests failed 16 of 17 and took 8.25 seconds rather than 0.9. Which test failed was not captured, and it has not recurred in 15 runs on that session's side or in this gate. It is unexplained, not fixed.

**Answered 2026-11-06:** no database outside development carries `seaql_migrations`, so adoption needs no rehearsal; it stays for development databases, and `operations/runbook.md` keeps the steps. `seaql_migrations` stays in place after adoption, unread, and `_sqlx_migrations` is the record. `down` files are kept, because `tests/qual26_migrations_test.rs` exercises them.

**Limits.** The counts are text searches. Effort is not estimated from a trial: no stage has been tried, and stage 3 is the largest and the most likely to find surprises. SQLX-1 was deleted from the queue on 2026-11-06 with no reason recorded, and restored the same day when it was asked for again.

