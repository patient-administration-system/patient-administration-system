# Comparison: Openfront Hospital

Researched 2026-11-02 from the public repository <https://github.com/openshiporg/openfront-hospital> (README, `docs/schema-map.md`, the model and mutation file lists, three model files read in full). Nothing was run: no clone, no build, no demo. Counts and claims below are the repository's own words unless marked *inferred*. The ideas it prompts are in [`plan.md`](../../../plan.md) §14 and `tasks.md` (series `OFH`).

## What it is

A self-hostable "open source Epic alternative" from the Openship ecosystem (one of about 30 sibling "Openfront" apps built from the same starter: hotel, salon, gym, pharmacy and so on). MIT licence, created 2026, about 30 commits, 1 star, 2 forks, pushed daily as of 2026-11-02. Next.js 16 and React 19, KeystoneJS 6 (GraphQL), PostgreSQL through Prisma, Tailwind 4, shadcn/ui, TypeScript. Deploys to Vercel or Railway with S3-compatible storage and mail. Password accounts with sessions. A separate worker process delivers an outbox.

It is a **whole-hospital** system (storefront, registration, scheduling, encounters, inpatient, orders, labs, prescriptions, claims, payments, inventory), with 63 data lists. This repository is an **administrative** PAS (`spec/02-scope.md`): the two overlap in the middle (patients, appointments, beds, admission, discharge, consent, messaging, audit) and diverge at both ends.

## Where it overlaps

| Area | Openfront Hospital | This repository |
|---|---|---|
| Appointment lifecycle | requested → confirmed → checked_in → in_visit → completed; cancelled, no_show; one `transitionAppointment` workflow | Fuller: series, standby, slot offers and proposals, rebook offers, outcomes, attendance, PIFU, group sessions, virtual visits, clinic templates |
| Public booking | Storefront `/care`, `/providers`, `/book`; visitor requests a type and a time; anonymous fee estimate (`publicEstimate`) | Patient-facing replies, links and offers; no anonymous self-booking page |
| Reminders and messages | `PatientMessage`, versioned `CommunicationTemplate`, per-patient `CommunicationPreference` (allowed / opted_out per channel and purpose) | Much fuller: reminder ladder, carriers with signed replies, opt-outs, WhatsApp rules, experiments, 10 locales |
| Beds and wards | Facility → Department → Ward → Room → Bed; `BedReservation` with `expiresAt` (active → consumed / released / expired); `BedOccupancyEvent` history; bed readiness available → reserved → occupied → cleaning | ADT with ward and bed state machine, pre-admit reservation, HL7 v2 and FHIR; no timed reservation expiry found *(inferred from a code search)* |
| Arrival | `CareQueueTicket` (waiting → called → in_care → completed, priority, position) from `arriveAppointment` | Not found: no front-desk arrival queue *(inferred)* |
| Consent | Versioned `PatientConsent` with a supersedes chain, policy version, capture method, evidence | Consent is in the domain (FR set) and the audit; a supersedes chain per policy version was not checked |
| Audit | `ClinicalAuditLog`; `PhiReadAudit` written by a GraphQL plugin (operation name, root fields, digest of the selection, result count, never values) | `audit_log` with BALP export (FR-190); single-record reads audited, **searches and batch reads are not** (`spec/09-api-surface.md`, NEW-9 "not covered") |
| Outbox | `ClinicalOutboxEvent` with attempts and dead letter, `IntegrationDeliveryAttempt`, inbound `IntegrationWebhookReceipt` (request digest), `MachineCredential` stored as digest only | Outbox with per-subscriber delivery, backoff, `failed` stays visible; receivers told to use `X-PAS-Event-Id` as the idempotency key |
| Data governance | `RetentionPolicy`, `RetentionDisposition` (append-only), `RecordHold`, encrypted `ExportJob` with expiry, `ExternalIdentityLink` | Retention schedule is a document (`governance/retention-schedule.md`); audit rows are never deleted; no record hold, no disposition record, no expiring export |
| Duplicate patients | `PatientRegistration.duplicateResolution` (no_candidate / create_new / link_existing) | Identity is delegated to person-service; `patient_replaced_by` and merge webhook |

## Where it is ahead, and worth studying

1. **Controlled writes with proof.** Records that matter have no generic create, update or delete. Every change goes through a named workflow carrying an `operationKey` (idempotency), an `expectedVersion` (optimistic concurrency) and a transition version, and writes an append-only `WorkflowTransition` row linked to the audit row and the outbox event. We found no `If-Match`, row version or expected-version check in the Rust API *(inferred from a code search)*: two staff editing one appointment can overwrite each other.
2. **Read auditing of searches.** `PhiReadAudit` records that a list or search resolved PHI, with counts and a digest, without storing values. It closes the gap we state ourselves in NEW-9.
3. **Legal hold and disposition.** A hold blocks disposal for a patient; a disposition is a recorded decision with a cutoff.
4. **Timed bed reservation** and a **bed occupancy event log** (who was where, when), separate from the movement record.
5. **Arrival queue** with a priority and a position, tied to the appointment and later to the encounter.
6. **Capability flags per area** (`canManageAppointments`, `canManageScheduling`, `canManageBilling`, `canAccessAllClinicalData`), with global clinical access an explicit grant, not implied by dashboard access.
7. **First administrator by a secret-protected, once-only bootstrap** that refuses to run when a user exists. Production start-up validates secrets.
8. **Schema map as documentation** (`docs/schema-map.md`): every list, who creates it, its states, in one table.
9. **Honest disclaimer.** "An installation record does not certify external EHR or laboratory connectivity"; "software workflows do not establish clinical, privacy, or regulatory certification." We make the same point in DTAC.

## Where we are ahead

- **Standards:** FHIR R5 and R4 `AuditEvent`, HL7 v2, SNOMED CT UK, NHS Wales data dictionary mappings. Openfront Hospital does not mention FHIR or HL7 at all.
- **Safety and assurance:** hazard log and closure rule, DTAC self-assessment, DPIA material, a personal-data inventory checked by a test, chaos tests, tolerances.
- **Fairness and measurement:** analytics, deprivation lookup, differencing budget, the no-show score held off until named people approve it.
- **Accessibility and language:** 10 locales with a visible-English check, axe, keyboard, reflow and text-spacing tests.
- **Quality gates:** database tests that cannot skip in CI, generated implementation status, spec-first requirement IDs cited in tests.
- **Architecture:** identity and workforce delegated to their own services; opaque UUIDs rather than a patient master copied into a second database.
- **Worldwide stance:** its data model is American in shape (amounts in cents, insurance claims and payer names, member and group numbers, MRN, copay-style fee estimate). It would need the national-assumption audit we are applying to ourselves (QUAL-40).

## Assessment

**Maturity.** Early and fast-moving: one star, 30 commits, MIT, a single visible push stream (a Claude-assisted workflow is visible in its files: `CLAUDE.md`, `ARCHITECT-PROMPT.md`, `MULTI-CLAUDE-SETUP.md`). Three test files are visible in `tests/` (family security, shared dashboard, store settings save), so most of its behaviour is untested in the repository as far as we can see. The depth of its design (transition evidence, controlled clinical commands, outbox, retention) is ahead of its test coverage. It is a design reference, not a dependency.

**Fit.** Not a replacement and not a fork candidate: different stack (Node, GraphQL, Prisma), different scope (clinical and billing), US-shaped, no standards interchange, no Welsh or any non-English UI evidence. It is a **consumer-facing storefront plus whole-hospital back office** where we are an **administrative core with interchange**.

**Risks in what it shows.** Clinical records (vitals, labs, prescriptions, care plans, claims) are exactly what `spec/02-scope.md` excludes, and each would trigger DCB0129/DCB0160 and, for software that informs care, medical-device regulation. Do not copy them. Anonymous booking creates a `Patient` row from a visitor: a data-quality and abuse surface we would have to weigh (duplicate creation, enumeration, rate limits; it has `RequestRateLimitBucket`).

**Licence.** MIT, so ideas and small structures may be borrowed with attribution if any code is copied. This work copies none; the ideas are re-specified from our own scope. Check the licence text in the repository before copying any file.

## Advice

Adopt, in this order (details and acceptance in `tasks.md`, series `OFH`):

1. **OFH-1** optimistic concurrency (`If-Match` / version) on appointments, waiting-list entries and admissions. Cheapest, closes a real lost-update hazard, and belongs in the hazard log.
2. **OFH-2** audit searches and batch reads (counts and a digest, never values). Closes the stated NEW-9 gap.
3. **OFH-3** record hold and retention disposition as data, so deletion and archive decisions are recorded, not only documented.
4. **OFH-4** timed bed reservation with expiry, and a bed occupancy event log.
5. **OFH-5** an arrival queue (check-in, called, in care), if the board's front desks want it.
6. **OFH-6** per-area capability flags and a once-only bootstrap, if Keycloak roles do not already express them.
7. **OFH-7** a schema map page generated from the entities, kept true by a test, like the implementation status.
8. **OFH-8** (decision) anonymous self-booking page: only after a safety and abuse review.
9. **OFH-9** (decision) encrypted, expiring data export jobs.

Do not adopt: clinical record lists, claims and payments, inventory, the "store" metaphor, password accounts, or a second patient master.

## Not checked

The live demo; the external documentation at docs.openship.org; the Prisma schema field by field; whether its access filters are correct; its test content beyond filenames; the security configuration validator; the licences of its dependencies. Our "not found" statements come from a text search of `src/` and `migrations/` and could miss a differently named feature.
