# Plan

Research and reasoning behind [`tasks.md`](tasks.md). Tasks are the commitments; this file is where ideas are weighed. Written 2026-10-18.

## 1. Where the monorepo stands

| Part | What it is | State |
|---|---|---|
| `patient-administration-system-with-rust` | Axum 0.8 + SeaORM + Postgres 18 server: ADT, appointments, waitlist and RTT, wards and beds, letters, billing, consent, audit, outbox, FHIR R5, HL7 v2, reminders. Identity (patients, workers) is delegated to upstream `person-service` and `worker-service` | Builds, clippy clean, 400+ tests passing against a real database |
| `patient-administration-system-frontend` | Loco 0.14 server-rendered operator pages (Tera + HTMX) | Could not start until 2026-10-18 (BUG-2); now passes its smoke tests |
| `patient-administration-system-ui-with-svelte` | SvelteKit 3 backend-for-frontend: Keycloak sign-in, patient and staff pages, admin template editor | New. Checked against real Keycloak and a mock API |
| `podman/` | PostgreSQL 18 and Keycloak 26 for development | Working; ports overridable |
| `docs/…/digital-technology-assessment-criteria/2.0` | NHS DTAC v2.0 self-assessment | Says the product would not pass today |

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
