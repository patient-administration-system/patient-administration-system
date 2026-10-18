# Patient Administration System (PAS) — Specification

Comprehensive specification-driven-development document for the Patient Administration System workspace. This file is the **single source of truth** for what the system is, how it must behave, and how it is architected.

> **Shipped baseline: v0.44.0 — Option C identity delegation.**
> Identity (`Patient`) and workforce (`Practitioner` + `PractitionerRole` + `Department`) are **fully delegated** to external HTTP services: `person-service` (`:5180`) and `worker-service` (`:5181`). v0.44 has shipped: the delegation landed across Phases 0 → 3e (docs → upstream clients → destructive migration → REST + FHIR proxies → PAS-owned typed handlers → HL7 v2 inbound + outbound MLLP publisher → bulk interchange → FHIR Schedule/Slot writes + Bundle Decision #4 enforcement). The PAS schema no longer holds `patients` / `practitioners` / `practitioner_roles` / `departments` tables.
>
> This is a *doctrine reversal* from v0.43 §2.3, which said "the PAS does not depend on the MPI." In v0.44 it does — those two services *are* the MPI realized — and the integration is **HTTP-only** (no Cargo dependency on either upstream crate).

This document replaces the former `plan.md` + `tasks.md` pair. Past release notes live in [`CHANGELOG.md`](CHANGELOG.md) (per-crate) and [`../CHANGELOG.md`](../CHANGELOG.md) (workspace). Detailed reference per topic lives under [`AGENTS/`](AGENTS/).

---

## 1. Identity & Purpose

The Patient Administration System (PAS) is the foundational, **non-clinical workflow service** for a hospital. It owns *when, where, what visit type, what bed, what bill*. It **delegates** the *who*: identity (`Patient`) lives in `person-service`, workforce (`Practitioner`) lives in `worker-service`. The PAS holds opaque foreign keys (`person_id`, `worker_id`) that reference those services. It does **not** own the *what diagnosis* either; clinical content (diagnoses, prescriptions, lab results, vitals, orders) belongs to the EMR.

The shipped system is a Rust Cargo workspace with three members, plus two **runtime HTTP dependencies** on services that ship from separate workspaces:

| Crate / Service                                 | Role                                                                | Port  | Stack                                |
|-------------------------------------------------|---------------------------------------------------------------------|-------|--------------------------------------|
| `patient-administration-system`                 | Workflow service. REST + FHIR R5 + HL7 v2 ADT lifecycle (HTTP + MLLP). | 8080 | Axum 0.7, sea-orm 1.1, utoipa 5.4 |
| `patient-administration-system-migrations`      | sea-orm migration crate shared by both PAS apps. CLI binary `pas-migrate`. | n/a | sea-orm-migration 1.1                |
| `patient-administration-system-frontend`        | Loco-rs read-mostly UI. Server-side Tera + HTMX + Lily Design System. | 5150 | Loco-rs 0.14.1, Tera, axum-extra cookies, reqwest |
| `person-service` *(HTTP dep, external)*    | Person identity, demographics, search, match, merge, FHIR R5 Person. | 5180 | (own crate; see its `spec.md`) |
| `worker-service` *(HTTP dep, external)*    | Worker (= practitioner) identity, demographics, search, match, merge, FHIR R5 Worker. | 5181 | (own crate; see its `spec.md`) |

Both PAS binary crates share the same PostgreSQL database. The migration crate is the single source of truth for the *PAS-owned* schema (encounters, admissions, appointments, billing, etc.); apply it once via `pas-migrate up` and both apps see it. **The PAS schema does not contain `patients`, `practitioners`, `practitioner_roles`, or `departments` tables** — those live in the upstream services' databases.

The PAS is **HTTP-coupled** to the two upstream services: every `person_id` / `worker_id` mention in a PAS API call results in a lookup against the matching upstream service. There is no local cache (Option C v1); operational availability of the PAS therefore depends on availability of both upstream services. See §5 invariant #1 and §6.8 for the operational implications.

---

## 2. Scope

### 2.1 In scope

- **Visit lifecycle** — Encounter, Admission, Transfer, Discharge, BedAssignment. Owns `person_id` (FK to upstream).
- **Identity proxy** — `/api/patients/*`, `/fhir/Patient/*`, and PID-bearing HL7 v2 triggers (A01/A02/A03/A04/A05/A06/A07/A08/A11/A12/A13/A21/A22/A23/A28/A38/A40 + DFT^P03 + SIU + MFN^M02-via-PID) all **proxy** to `person-service`. The PAS parses the wire, performs dedup-or-create against the upstream, and stores only the returned `person_id` on its local rows. Bulk interchange (`/api/patients/export.{json,xml,tsv,csv}` + `/api/patients/import`) likewise proxies upstream.
- **Workforce proxy** — `/api/practitioners/*`, `/fhir/Practitioner/*`, and MFN^M02 inbound/outbound all proxy to `worker-service`. The PAS stores only the returned `worker_id` on its local rows (`appointments.worker_id`, `schedules.owner_worker_id`, etc.).
- **Multinational national-government healthcare identifiers** — handled entirely by `person-service` (UK NHS Number, France NIR, España TSI, Ireland IHI, Northern Ireland HCN, US SSN/DL/Passport/Other). The PAS never validates an identifier itself; the upstream returns 400 on bad input and the PAS proxies the error envelope.
- ADT (Admission / Discharge / Transfer) with atomic bed allocation.
- Scheduling: slots, appointments, booking, cancellation, check-in, completion, no-show, bulk slot generation.
- Waitlist + RTT (Referral-To-Treatment) clock with per-pathway breach threshold.
- Resources: facilities, wards, rooms, beds, ward occupancy snapshots, bed-status state machine.
- Communications: Tera-rendered patient letters with strict required-variable enforcement.
- Episode-of-care billing: accounts, charges, invoices, payments. Multi-currency via ISO 4217.
- Consent records (GDPR/HIPAA-aligned) with explicit `is_active(today)` evaluation.
- Privacy: masked patient view, GDPR export, consent CRUD + revoke.
- Audit log written transactionally with every state-changing call.
- Transactional outbox + background dispatcher for domain events.
- Full-text patient search (Tantivy) kept in lockstep with CRUD.
- RESTful API (Axum) with response envelope and bearer-token auth (opt-in).
- FHIR R5 surface: read/write for `Patient`, `Encounter`, `Appointment`; reads for `Practitioner`, `Schedule`, `Slot`, `Location`; batch + transaction `Bundle` writes.
- Bulk patient interchange in JSON / XML / TSV / CSV via a flat `PatientRow` projection.
- HL7 v2 ADT — A01 / A02 / A03 / A04 / A05 / A06 / A07 / A08 / A11 / A12 / A13 / A21 / A22 / A23 / A28 / A38 / A40 — over HTTP **and** MLLP TCP. Inbound exact-MRN dedup on A01 / A28. **A04 register outpatient** (v0.28) is the orthogonal ambulatory complement to A01: dedup-or-create patient from PID, open an `Encounter` in `Arrived` status with class derived from PV1-2 (`E` → Emergency, anything else → Outpatient), no bed allocation. v0.29 closes the loop on the outbound side: REST `create_encounter` for Outpatient / Emergency classes emits `EncounterRegistered → ADT^A04`, source-gated on `hl7v2_a04`. **Batch envelope** (`FHS`/`BHS`/`BTS`/`FTS`, v0.6) carries up to 1000 messages per transmission. **A40 patient-merge** (v0.18) couples to the v0.11 `replaced_by` tombstone surface: inbound ADT^A40 looks up the source by MRG-1.1 MRN and applies the same merge logic as the REST `/api/patients/{id}/merge-into/{target_id}` endpoint. Outbound publisher when `HL7V2_OUTBOUND_PEER` is set (source-gated so REST-driven edits don't echo).
- HL7 v2 DFT — P03 (post detail financial transaction) — over the same HTTP + MLLP surface (v0.19 + v0.20). Inbound `POST /api/hl7/v2/dft` accepts MSH+EVN+PID+FT1{1..N}; the patient is dedup-or-created from PID, an open billing account is found or auto-created in the FT1-11.2 currency, and **one charge per FT1 segment** is posted in a single DB transaction (v0.20 — all FT1 must share the same currency). Outbound `Hl7v2MllpPublisher` emits one `DFT^P03` per `ChargePosted` outbox event, source-gated on `hl7v2_p03`.
- HL7 v2 MFN — M02 (Master File Notification — Staff) — over the same HTTP + MLLP surface (v0.24 + v0.25). Inbound `POST /api/hl7/v2/mfn-staff` accepts MSH+MFI+(MFE+STF){1..N}; each MFE+STF pair adds (`MAD`), updates (`MUP`), or soft-deletes (`MDL`) a practitioner row, atomic per message. The EMR's staff id (MFE-4 or STF-1) is stored as an `Identifier { type: Other, system: "urn:hl7v2:staff:id" }` so subsequent `MUP` / `MDL` messages can locate the same PAS row. Outbound (v0.25) `Hl7v2MllpPublisher` emits one `MFN^M02` per `PractitionerCreated` / `PractitionerUpdated` / `PractitionerDeactivated` outbox event (source-gated on `hl7v2_mfn_m02` to avoid boomerang); REST + FHIR practitioner CRUD handlers were updated in v0.25 to write the outbox + audit entries they previously bypassed.
- HL7 v2 MFN — M05 (Master File Notification — Patient Location) — over the same HTTP + MLLP surface (v0.26 + v0.27). Inbound `POST /api/hl7/v2/mfn-location` accepts MSH+MFI+(MFE+LOC){1..N}; each MFE+LOC pair adds (`MAD`), updates (`MUP`), or soft-deletes (`MDL`) a bed row, atomic per message. LOC-1 is `PL`-typed: LOC-1.1 = parent room code, LOC-1.3 = bed code; LOC-2 = bed name. `MDL` flips the bed to `BedStatus = OutOfService` via the same operator-bypass (`set_status_unchecked`) used for ADT^A13 cancel-discharge. Outbound (v0.27) `Hl7v2MllpPublisher` emits one `MFN^M05` per `BedCreated` / `BedUpdated` / `BedRetired` outbox event (source-gated on `hl7v2_mfn_m05` to avoid boomerang); REST `create_bed` / `update_bed` (new in v0.27) / `set_bed_status` (when flipping to `OutOfService`) all write the matching audit + outbox entries.
- HL7 v2 SIU — S12 (book) / S13 (reschedule) / S14 (modify) / S15 (cancel) — over HTTP and MLLP TCP (v0.16 + v0.17). Inbound endpoints at `/api/hl7/v2/schedule-book`, `/schedule-reschedule`, `/schedule-modify`, `/schedule-cancel` accept SCH+PID; the filler appointment id is the PAS UUID and travels end-to-end so the EMR can dedupe on it. Outbound mapping in `Hl7v2MllpPublisher`: `AppointmentBooked → SIU^S12`, `AppointmentRescheduled → SIU^S13`, `AppointmentModified → SIU^S14`, `AppointmentCancelled → SIU^S15`, all source-gated so EMR-originated SIU isn't echoed back as PAS-originated.
- Complete OpenAPI 3.1 spec covering every REST + FHIR + HL7 v2 handler.
- Operational dashboard with HTMX live-refresh and Lily Design System markup.
- Sibling Loco-rs front-end with patient pages, ward detail, RTT cockpit, and three CSRF-protected write forms (admit / book / letter) that proxy back through the PAS REST API.

### 2.2 Out of scope (deferred)

Listed so a developer knows **not** to build them:

- Real JWT / OAuth identity-provider integration (bearer-token middleware exists; that's it).
- gRPC server (scaffolded then removed in v0.3.1 dep audit).
- Fluvio event producer (stub returns `Error::Streaming`).
- (none — recurring appointment series landed in v0.9.0.)
- Production SMS gateway integrations (Twilio, MessageBird, …) — v0.8 ships the trait + `LogSmsProvider`; consumers add real gateways by implementing `SmsProvider`.
- HL7 v2 ORM (order entry) — orders are clinical, out of PAS scope.
- DICOM Modality Worklists.
- Probabilistic patient matching, fuzzy duplicate detection, record merging — sister MPI crate's job.
- Web UI inside the PAS Axum binary — that's what `patient-administration-system-frontend` is for.

### 2.3 Boundary with the upstream identity services

The PAS **fully delegates** identity decisioning, storage, search, matching, and merging to two external HTTP services that ship from separate workspaces:

| Concern | Owner | Endpoint family |
|---|---|---|
| Person CRUD | `person-service` | `POST/GET/PUT/DELETE /api/persons[/{id}]` |
| Person search (full-text + fuzzy + phonetic) | `person-service` | `GET /api/persons/search` |
| Person match + dedup | `person-service` | `POST /api/persons/{match,check-duplicates,deduplicate}` |
| Person merge | `person-service` | `POST /api/persons/merge` |
| Person FHIR R5 | `person-service` | `/fhir/Person/*` |
| Worker (= Practitioner) CRUD / search / match / merge / FHIR | `worker-service` | symmetric to person — `/api/workers/*` + `/fhir/Worker/*` |

The PAS exposes a **compatibility-shim surface** so external callers see the historical `/api/patients/*`, `/api/practitioners/*`, `/fhir/Patient/*`, `/fhir/Practitioner/*` URLs unchanged. Every handler on those routes proxies to the matching upstream endpoint and translates the resource shape (Patient ↔ Person, Practitioner ↔ Worker).

The integration seam between the PAS and the upstream services is two opaque UUIDs:

- **`person_id: Uuid`** replaces every former `patient_id` column on every PAS-owned table (encounters, admissions, transfers, discharges, bed_assignments, appointments, appointment_series, waitlist_entries, rtt_pathways, accounts, charges, invoices, payments, coverages, consents, generated_letters, referrals).
- **`worker_id: Uuid`** replaces every former `practitioner_id` column on every PAS-owned table (encounters, admissions, transfers, discharges, appointments, schedules).

The columns carry no DB-level foreign key (the referenced row lives in another service's database). Validity is enforced at write time: the PAS's REST/FHIR/HL7 handler resolves the `person_id` against `person-service` (e.g. via `GET /api/persons/{id}`) before persisting; missing rows return `404 + NOT_FOUND` from the PAS.

**Merge-back integration.** Merges are decided and executed entirely by `person-service` / `worker-service`. The PAS receives merge notifications via **inbound webhook** at `POST /api/upstream/person-merged` / `POST /api/upstream/worker-merged` (HMAC-SHA256 verified via `PERSON_SERVICE_WEBHOOK_SECRET` / `WORKER_SERVICE_WEBHOOK_SECRET`). On receipt the PAS rewrites every PAS-owned row whose `person_id`/`worker_id` matched the merge `source_id` to point at `target_id`, audit-logs the rewrite (`action = "upstream_person_merged"` / `"upstream_worker_merged"`), and **does not** emit its own outbox event — the upstream is the system of record for the merge.

The `/api/patients/{id}/merge-into/{target_id}` endpoint historically owned by the PAS is **removed** in v0.44.0; clients that need to trigger a merge call `POST /api/persons/merge` on `person-service` directly. The same applies to the merge tombstone surface (`/api/patients/{id}/replaces`) and the FHIR `Patient.link[type = "replaced-by"]` shape — those become responsibilities of `person-service`'s `/fhir/Person/{id}` representation.

---

## 3. Stakeholders & Use Cases

| Stakeholder           | Primary use cases                                                                      |
|-----------------------|----------------------------------------------------------------------------------------|
| Hospital admin staff  | Register patients, manage facility hierarchy, run reports.                             |
| Front-desk clerks     | Book appointments, check patients in, capture cancellations / no-shows.                |
| Ward nurses           | View ward occupancy, transfer patients between beds, discharge.                        |
| Billing team          | Open accounts, post charges, finalize invoices, record payments.                       |
| Compliance / privacy  | Query audit log, satisfy GDPR subject-access requests, manage consent.                 |
| EMR integration       | Receive ADT messages, send ADT messages, exchange FHIR R5 Bundles.                     |
| External labs / EHRs  | Bulk-import patient demographics; consume FHIR R5 reads.                               |
| Ops / SRE             | Monitor health, watch outbox publishing, inspect dashboard, scale replicas.            |
| AI agents / scripts   | Drive the REST + OpenAPI surface; integration via Swagger UI explorer.                 |

---

## 4. Functional Requirements

### 4.0 Patient identity (delegated)

Every endpoint listed below is a **thin proxy** over `person-service`. The PAS handler maps the legacy `/api/patients/*` URL to the matching `/api/persons/*` upstream URL, translates the response (`Person` → `Patient` shape on the wire so historical clients see no change), and surfaces upstream errors via the standard PAS envelope.

| PAS-exposed endpoint | Upstream call | Notes |
|---|---|---|
| `POST /api/patients` | `POST /api/persons` | Body translated; returned `person.id` surfaces as `patient.id` to the client. |
| `GET /api/patients/{id}` | `GET /api/persons/{id}` | Identity is shared — `{id}` *is* the upstream person id. |
| `PUT /api/patients/{id}` | `PUT /api/persons/{id}` | |
| `DELETE /api/patients/{id}` | `DELETE /api/persons/{id}` | Upstream owns soft-delete semantics. |
| `GET /api/patients/search` | `GET /api/persons/search` | Query string passed through; result shape translated. |
| `GET /api/patients/{id}/masked` | `GET /api/persons/{id}/masked` | |
| `GET /api/patients/{id}/export` | `GET /api/persons/{id}/export` | GDPR export now combines upstream demographics + PAS-owned related rows (encounters, appointments, billing, consents). PAS handler concatenates. |
| `GET /api/patients/{id}/audit` | `GET /api/persons/{id}/audit` *(proxied)* + PAS-local audit rows merged | Audit on demographics lives upstream; audit on PAS-owned actions (admit, book, charge) lives in PAS `audit_log`. The handler returns both, time-sorted. |

#### Removed in v0.44.0

- **`POST /api/patients/{id}/merge-into/{target_id}`** — merges happen at `person-service` (`POST /api/persons/merge`). Clients call upstream directly.
- **`GET /api/patients/{id}/replaces`** — superseded by the upstream `/fhir/Person/{id}` `link[type = "replaced-by"]` representation.
- **Local Tantivy patient index** — `person-service` owns search.
- **Local exact-MRN dedup on HL7 v2 A01/A28** — handled by the upstream's `POST /api/persons/match` (or `check-duplicates`) call performed inside the proxy handler.

#### PAS-owned related-row endpoints (still local)

These are unchanged — they read PAS-owned tables keyed by `person_id`:

- `GET /api/patients/{id}/encounters`
- `GET /api/patients/{id}/appointments`
- `GET /api/patients/{id}/appointment-series`
- `GET /api/patients/{id}/consents`
- `GET /api/patients/{id}/coverages`

#### Merge propagation (inbound webhook)

When `person-service` completes a merge, it POSTs to `/api/upstream/person-merged` on the PAS:

```json
{ "source_id": "<uuid>", "target_id": "<uuid>", "at": "..." }
```

HMAC-SHA256 signed via `X-Upstream-Signature` keyed by `PERSON_SERVICE_WEBHOOK_SECRET`. Handler rewrites every PAS-owned row whose `person_id == source_id` to `target_id` in one DB transaction, writes one `audit_log` row per rewritten table (`action = "upstream_person_merged"`), and ACKs 200. Replay-safe on `X-Upstream-Event-Id`.

#### 4.0.1 Multinational national-government healthcare identifiers (delegated)

`person-service` owns identifier types and per-country format validation (NHS Mod 11, NIR Mod 97 with Corsica fix-up, TSI alphanumeric envelope, IHI, HCN). The PAS surface forwards identifier objects verbatim in the request body to `/api/persons` and surfaces upstream `400 + invalid` envelopes as `400 + VALIDATION` in the PAS envelope. No identifier validation logic lives in the PAS as of v0.44.0; the v0.15 `src/validation/identifier.rs` family is removed.

> **DECISION (default, override before Phase 1):** The PAS does **not** cache identifier validation locally. Every identifier-bearing PAS call costs one upstream round-trip. If latency becomes a concern, add a 1 min in-process LRU cache on `GET /api/persons/{id}` in Phase 7.

### 4.1 ADT (Admission / Discharge / Transfer)

- **Admit** (`POST /api/admissions`): given `(patient_id, bed_id)`, open an Inpatient `Encounter`, lock the bed with `SELECT … FOR UPDATE`, assert `BedStatus::Available`, transition to `Occupied`, insert an active `BedAssignment` (`released_at = NULL`), insert an `Admission` row, write `audit_log` + `outbox_events` (`EncounterAdmitted`) in the same transaction.
- **Transfer** (`POST /api/admissions/:id/transfer`): close the old `BedAssignment` (`released_at = now`), flip the old bed to `Cleaning`, lock the new bed, assert `Available`, flip to `Occupied`, insert a new active `BedAssignment`, insert a `Transfer` row, write audit + outbox (`EncounterTransferred`).
- **Discharge** (`POST /api/admissions/:id/discharge`): close the active `BedAssignment`, flip the bed to `Cleaning`, insert a `Discharge` row, transition the encounter to `Finished`, write audit + outbox (`EncounterDischarged`).
- **No silent fallback**: if the target bed is not `Available`, the operation fails with `409 Conflict`. The PAS never auto-picks a substitute bed.

### 4.2 Scheduling

- A `Slot` is a pre-generated `(schedule_id, start_datetime, end_datetime, status)` window.
- **Book** (`POST /api/slots/:id/book`): lock the slot row, assert `SlotStatus::Free`, run per-patient overlap check (half-open intervals; reject `Cancelled` + `NoShow` appointments), transition slot to `Busy`, insert `Appointment(Booked)`, write audit + outbox (`AppointmentBooked`).
- **Cancel** (`POST /api/appointments/:id/cancel`): reverse the slot status (`Busy → Free`) in one transaction; record a `CancellationReason`.
- **Check-in** (`POST /api/appointments/:id/check-in`): `Booked → Arrived`.
- **Complete** (`POST /api/appointments/:id/complete`): `Arrived → Fulfilled`.
- **No-show** is a terminal status; reachable from `Booked`.
- **Bulk slot generation** (`POST /api/schedules/:id/slots/bulk`): `{start_datetime, end_datetime, slot_minutes}` → consecutive `Free` slots.
- Overlap semantics are **half-open**: an appointment ending at 10:00 does not overlap one starting at 10:00. Implemented predicate: `existing.start < new.end AND existing.end > new.start`.
- **Recurring series (v0.9)**: a new `AppointmentSeries` aggregate represents a *plan* (patient, service, start, duration, recurrence rule) that expands to N concrete `Appointment` rows at create time. The rule is a deliberately narrow RFC 5545 subset: `FREQ` = `Daily`/`Weekly`/`Monthly`, `INTERVAL >= 1`, optional weekly `BYDAY`, exactly one of `COUNT` or `UNTIL` for termination. Hard cap `MAX_OCCURRENCES = 200`. Each generated occurrence is a normal `Appointment` row (`status = Booked`, `slot_id = NULL`, `series_id = <series.id>`); the per-patient overlap predicate still applies. **Create is atomic**: any per-patient overlap on any occurrence rolls back the whole transaction with `409 + diagnostic` naming the offending datetime + appointment id. **Preview** (`POST /api/appointment-series/preview`) is a pure dry-run — no DB writes — so the UI can confirm the schedule before commit. **Cancel** (`POST /api/appointment-series/{id}/cancel`) flips the series to `Cancelled` and walks every linked occurrence, cancelling only the `Proposed` / `Booked` rows (terminal statuses untouched). Endpoints: `POST /api/appointment-series/preview`, `POST /api/appointment-series`, `GET /api/appointment-series/{id}`, `POST /api/appointment-series/{id}/cancel`, `GET /api/patients/{id}/appointment-series`.

### 4.3 Waitlist + RTT clock

- **Waitlist add** (`POST /api/waitlist`): `(patient_id, target_service, priority, referral_id?)`. `Priority` ordering: `Routine < Urgent < TwoWeekWait < Emergency`.
- **RTT start** (`POST /api/rtt/start`): create `RTTPathway(Active)` with `breach_weeks = 18` by default. Append a `Started` event.
- **Pause / resume / stop**: append `Paused(reason)` / `Resumed` / `Stopped(reason)` events.
- **Weeks waiting** (`GET /api/rtt/:id/weeks-waiting`): runs `compute_active_weeks(events, now)`: sum of unpaused intervals from `Started`/`Resumed` to next `Paused`/`Stopped` (or `now` if open), floor-divided by `SECONDS_PER_WEEK`. Negative deltas (clock skew) clamp to 0.
- **Breach predicate**: `compute_active_weeks(...) > pathway.breach_weeks`. **Strict** comparison — at exactly `breach_weeks`, the pathway is not yet breaching.
- **Breaches list** (`GET /api/waitlist/breaches`): scan active pathways, return any whose `weeks_waiting > breach_weeks`.
- **Append-only event log**: `event_at` monotonically non-decreasing per pathway; `Started` is first; `Stopped` (if present) is last; `Paused`/`Resumed` alternate. Enforced by `src/validation/`.

### 4.4 Resources

- Hierarchy: `Facility → Ward → Room → Bed`. Each is its own table; loaded by id.
- **Ward occupancy** (`GET /api/wards/:id/occupancy`): counts beds by status.
- **Bed status change** (`PUT /api/beds/:id/status`): state-machine guarded.
- Bed state machine:
  - `Available → {Occupied, Reserved, OutOfService}`
  - `Occupied → {Cleaning, OutOfService}` — **never directly to `Available`**.
  - `Cleaning → {Available, OutOfService}`
  - `Reserved → {Occupied, Available, OutOfService}`
  - `OutOfService → Available`
  - Same-state transitions explicitly rejected.

### 4.5 Billing

- **Money is `rust_decimal::Decimal` + `Iso4217`. Never `f64`.** Currency mismatch on `Money::try_add` returns `Error::Validation`.
- **Open account** (`POST /api/accounts`): one `Open` account per patient invariant (re-open while open → `409 Conflict`). Currency is fixed at creation.
- **Post charge** (`POST /api/charges`): amount must match account currency. Money fields on the wire are split into `amount_value` (string decimal) + `amount_currency` (ISO 4217 code) so JSON numbers can never lose precision.
- **Finalize invoice** (`POST /api/invoices`): sum referenced charges; status `Draft → Finalized`.
- **Post payment** (`POST /api/payments`): partial payments allowed; invoice flips between `Finalized → PartiallyPaid → Paid`.
- **Coverage** (v0.10): insurance / self-pay / other-payer record per patient, with optional linkage to a billing `Account`. Status enum: `Active` / `Cancelled` / `Draft` / `EnteredInError` (FHIR R5 aligned). Kind enum: `Insurance` (default) / `SelfPay` / `Other`. Fields: `payor_name` (required), `policy_number` (required), optional `payor_identifier`, optional `group_number`, `relationship` (default `"self"`; supports `"spouse"`, `"child"`, etc. for non-self subscribers), optional `subscriber_id` (defaults to `patient_id`), `start_date`, optional `end_date`. **No state machine** — status flips are intentional and operator-driven (e.g. clerk corrects a wrong-policy-number row via `EnteredInError`). **No `deleted_at`** — retirement is via `status = Cancelled`; `DELETE /api/coverages/{id}` is a soft-cancel. Endpoints: `POST /api/coverages`, `GET/PUT/DELETE /api/coverages/{id}`, `GET /api/patients/{id}/coverages`, `GET /api/accounts/{id}/coverages`. FHIR R5 read at `GET /fhir/Coverage/{id}` (write through `POST /fhir` Bundle deferred to a follow-up). Partial unique index `(patient_id, payor_name, policy_number) WHERE status = 'active'` prevents active-row dupes. Outbox events: `CoverageCreated` / `CoverageUpdated` / `CoverageCancelled`.

### 4.6 Communications

- **Template create** (`POST /api/letter-templates`): subject + Tera body + `required_variables: Vec<String>` + allowed `channels`.
- **Generate** (`POST /api/letters/generate`): renders subject + body via Tera with the patient object as a top-level context variable plus any keys in `extra`. **Strict missing-variable check**: any value in `template.required_variables` not present in the render context → `400 VALIDATION`.
- **Delivery status** (`POST /api/letters/:id/sent`, `/failed`): lets an external mailer mark whether the letter went out. Both write audit entries. These remain the primary path for `Print` and `Email` — the PAS does not auto-dispatch those channels.
- **SMS auto-send (v0.8)**: when a letter is generated with `channel == Sms` *and* the configured [`SmsProvider`](#) reports `is_enabled()`, `CommunicationService::generate_letter` also looks up the patient's first `ContactPoint { system: Phone }`, calls `provider.send(phone, rendered_body)`, and flips the row to `Sent` (stamping `sent_at`) or `Failed` accordingly. An `audit_log` entry with action `send_sms_ok` / `send_sms_failed` / `send_sms_skipped_no_phone` records what happened. Two first-party providers ship: `NoopSmsProvider` (default — `is_enabled() = false`, behavior matches v0.7), `LogSmsProvider` (logs every outbound message at `tracing::info!(target: "pas::sms")`, useful for dev/test). Production SMS gateway integrations (Twilio, MessageBird, …) are consumer-provided by implementing the `SmsProvider` trait — same pattern as `EventPublisher`.

### 4.7 Privacy

- **Masked view** (`GET /api/patients/:id/masked`): phone middle digits, postal-code suffix, email local-part, address line 1, and identifier tails are masked.
- **GDPR export** (`GET /api/patients/:id/export`): JSON dump of the patient and all related rows (encounters, appointments, consents, etc.) — suitable as a subject-access response.
- **Consent CRUD**: `POST /api/patients/:id/consents`, `GET /api/patients/:id/consents`, `POST /api/consents/:id/revoke`. All writes recorded in audit.
- **`Consent::is_active(today)`**: returns `false` if status is not `Active`, `granted_date > today`, or `expiry_date < today`. **Boundary rule**: on the expiry day itself the consent is still active.
- **`require_consent(...)` helper**: available for handlers that need a gate; not wired into every flow.

### 4.8 Interoperability

#### 4.8.1 Bulk interchange (v0.2)

- Flat `PatientRow` projection shared by JSON / XML / TSV / CSV. Field order is fixed (part of the TSV contract): `id, mrn, family_name, given_names, gender, birth_date, phone, email, line1, city, postal_code, country, active`.
- `GET /api/patients/export.{json,xml,tsv,csv}` capped at 10,000 active patients.
- `POST /api/patients/import` sniffs `Content-Type` to pick format. **Idempotent**: rows whose `id` exists are skipped, not overwritten. Response `{ inserted, skipped, failed }`.
- Lossy fields documented in [`AGENTS/interchange.md`](AGENTS/interchange.md). For higher-fidelity exchange use the FHIR R5 endpoints or the native PAS JSON shape (`GET /api/patients/:id/export`).

#### 4.8.2 FHIR R5

- Read/write: `Patient`, `Encounter`, `Appointment`, `Practitioner`, `Schedule`, `Slot` (the last three v0.21). Each has the `POST /fhir/<Type>` + `GET/PUT/DELETE /fhir/<Type>/{id}` quartet. Practitioner DELETE is **soft via the FHIR `active` flag** (the row remains addressable at GET); Schedule and Slot DELETEs are **hard deletes** (invariant §5.3 keeps soft-delete to patients / encounters / appointments). PUT treats the URL id as canonical and **ignores any client-supplied body id** before parsing — a placeholder like `"ignored-client-id"` no longer fails.
- Read at `/fhir/Coverage/{id}`; **write via `POST /fhir` Bundle entries** as of v0.13 (`FhirCoverage::into_domain` parses the wire shape; `create_coverage_in_db` dispatches to `CoverageRepository::create`).
- Reads only: `Location` (PAS `Bed`).
- Collection Bundle: `GET /fhir/Patient?_count=N`.
- **Bundle writes**: `POST /fhir` accepts `type: batch` or `type: transaction`. Entries may carry `resourceType` ∈ {`Patient`, `Encounter`, `Appointment`, `Coverage`, `Practitioner`, `Schedule`, `Slot`} (last three v0.23); any other type returns `400 Bad Request: unsupported resourceType …` in the per-entry response (batch) or rolls the whole transaction back (transaction).
  - `batch`: entries dispatched independently against the DB pool; per-entry `response.status`.
  - `transaction`: **genuinely all-or-nothing**. Every entry runs inside one `sea_orm::DatabaseTransaction`. Any failure → roll back the whole bundle and return `400 + OperationOutcome` whose `diagnostics` names the offending entry index. Search-index updates are deferred until after commit so rolled-back rows are never Tantivy-visible.
- Errors return FHIR `OperationOutcome` JSON with the same status mapping as REST.

#### 4.8.3 HL7 v2 ADT (v0.3 + v0.4)

- Standard delimiter set only (`|^~\&`). Custom delimiters return `400 + AR`.
- Segments: `MSH`, `EVN`, `PID`, `PV1`.
- HTTP endpoints (all respond with a v2 ACK envelope):
  - `POST /api/hl7/v2/parse` (echoes structured JSON).
  - `POST /api/hl7/v2/batch` (v0.6 — accepts an `FHS`/`BHS`/`BTS`/`FTS` envelope; see below).
  - `POST /api/hl7/v2/patient` (ADT^A28 add-person).
  - `POST /api/hl7/v2/update` (ADT^A08 update patient info — *v0.4*).
  - `POST /api/hl7/v2/admit` (ADT^A01: create + admit; PV1-3.3 = bed code).
  - `POST /api/hl7/v2/pre-admit` (ADT^A05: reserve bed + create Planned inpatient encounter; no admission or bed_assignment row — *v0.32*).
  - PAS-native `POST /api/admissions/pre-admit` invokes the same `AdtService::pre_admit` without a `source` tag, allowing the outbound publisher to emit `ADT^A05` to downstream peers — *v0.33*.
  - `POST /api/hl7/v2/cancel-pre-admit` (ADT^A38: release the bed reservation and cancel the Planned inpatient encounter set by a prior A05 — *v0.34*).
  - `POST /api/hl7/v2/leave-start` (ADT^A21: patient goes on leave of absence; encounter InProgress → OnLeave; bed remains Occupied — *v0.36*).
  - `POST /api/hl7/v2/leave-end` (ADT^A22: patient returns from leave of absence; encounter OnLeave → InProgress — *v0.36*).
  - `POST /api/hl7/v2/delete-patient` (ADT^A23: soft-delete a patient record sent in error; refuses if the patient has any open admission — *v0.39*).
  - The existing REST `DELETE /api/patients/{id}` now applies the same safety check + writes the `PatientDeleted` outbox event without a `source` tag, allowing the outbound publisher to emit `ADT^A23` to downstream peers — *v0.40*.
  - PAS-native `POST /api/admissions/{id}/leave-start` and `/leave-end` invoke the same service methods without a `source` tag, allowing the outbound publisher to emit `ADT^A21` / `ADT^A22` to downstream peers — *v0.37*.
  - PAS-native `POST /api/admissions/cancel-pre-admit` invokes the same `AdtService::cancel_pre_admit` without a `source` tag, allowing the outbound publisher to emit `ADT^A38` to downstream peers — *v0.35*.
  - `POST /api/hl7/v2/register` (ADT^A04: register outpatient / emergency, no bed; PV1-2 = `E` → Emergency else Outpatient — *v0.28*).
  - `POST /api/hl7/v2/change-to-inpatient` (ADT^A06: promote the most-recent active ambulatory encounter to Inpatient + bed allocation — *v0.41*).
  - `POST /api/hl7/v2/change-to-outpatient` (ADT^A07: demote the open inpatient admission to Outpatient + release the bed; encounter stays InProgress — *v0.43*).
  - PAS-native `POST /api/admissions/change-to-inpatient` invokes the same `AdtService::change_to_inpatient` without a `source` tag, allowing the outbound publisher to emit `ADT^A06` to downstream peers — *v0.42*.
  - `POST /api/hl7/v2/transfer` (ADT^A02: find open admission by MRN, transfer to PV1-3.3 bed).
  - `POST /api/hl7/v2/discharge` (ADT^A03: find open admission by MRN, discharge).
  - `POST /api/hl7/v2/cancel-admit` (ADT^A11: cancel the open admission — *v0.4*; outbound retrofitted in *v0.38* to source-gate against echo).
  - PAS-native `POST /api/admissions/{id}/cancel-admit` invokes the same `AdtService::cancel_admission` without a `reason` tag, allowing the outbound publisher to emit `ADT^A11` to downstream peers — *v0.38*.
  - `POST /api/hl7/v2/cancel-transfer` (ADT^A12: undo the most-recent transfer; restore patient to origin bed — *v0.30*).
  - PAS-native `POST /api/admissions/{id}/cancel-transfer` invokes the same `AdtService::cancel_transfer` without a `source` tag, allowing the outbound publisher to emit `ADT^A12` to downstream peers — *v0.31*.
  - `POST /api/hl7/v2/cancel-discharge` (ADT^A13: reinstate the most-recent discharge — *v0.4*).
  - `POST /api/hl7/v2/merge` (ADT^A40: merge source patient identified by MRG-1.1 MRN into survivor identified by PID — *v0.18*).
  - `POST /api/hl7/v2/dft` (DFT^P03: post a detail financial transaction — *v0.19*).
  - `POST /api/hl7/v2/mfn-staff` (MFN^M02: master file notification — practitioner add / update / soft-delete; atomic per message — *v0.24*).
  - `POST /api/hl7/v2/mfn-location` (MFN^M05: master file notification — bed add / update / retire; atomic per message — *v0.26*).
- ACK codes: `AA` (accept), `AE` (parsed but rejected by application), `AR` (parse failure).
- **Exact-MRN dedup on A01 / A28**: PID-3.1 lookup via Postgres JSONB `@>`. If a non-deleted patient with that MRN exists and the matched row actually carries that MRN type, the existing patient is reused — no duplicate insert, no audit, no Tantivy re-index. AA ACK reports `matched existing patient <uuid>` in MSA-3.
- **A08 merge semantics** (v0.4): the inbound PID **overwrites** `identifiers`, `name`, `telecom`, `gender`, `birth_date`, `addresses`, `updated_at`; the row's `id`, `mpi_id`, `additional_names`, `deceased*`, `emergency_contacts`, `marital_status`, `created_at`, `active` are **preserved** (PID has no field for them). Patient must already exist (look-up by PID-3.1) — otherwise AE. Single DB transaction writes the patient row + `audit_log` (`action = "update_via_hl7v2_a08"`) + `outbox_events` (`event_type = "PatientUpdated"`, payload includes `source: "hl7v2_a08"`). Tantivy re-index after commit. AA ACK includes `updated patient <uuid>` in MSA-3.
- **A11 semantics** (v0.4): locate the patient's currently-open admission by PID-3.1. Single DB transaction: release the active `BedAssignment`, flip the bed to `Cleaning` (regular `Occupied → Cleaning` transition — same as discharge), move the encounter to `Cancelled` (allowed from `InProgress` by the state machine). The `admissions` row is **preserved** so the erroneous admit stays visible in patient history. Outbox event `EncounterCancelled` with `reason: "hl7v2_a11"`.
- **A13 semantics** (v0.4): locate the patient's most-recently-discharged admission by PID-3.1. Lock the original bed; admit reinstatement is only allowed when its status is `Available` or `Cleaning` — anything else (Occupied / Reserved / OutOfService) means another patient now owns it and the discharge cannot be safely undone (AE). Single DB transaction: delete the `discharges` row, force-flip the bed to `Occupied` (state-machine **bypass** via `BedRepository::set_status_unchecked`), force-set the encounter back to `InProgress` (state-machine **bypass** via `EncounterRepository::set_status_unchecked` — `Finished` is normally terminal), insert a fresh active `BedAssignment` for the original bed. Both bypasses are the **only** documented exceptions to invariant §5.4 and are gated to the cancel-discharge flow. Outbox event `EncounterDischargeCancelled`.
- **HL7 v2 escape sequences** (`\F\` `\S\` `\T\` `\R\` `\E\`) honored at the domain↔wire boundary. Unknown vendor extensions pass through verbatim.
- **PID-29 / PID-30 deceased** (v0.22): `patient_from_pid` reads PID-29 (`YYYYMMDDHHMMSS` → `Patient.deceased_datetime`) and PID-30 (`Y`/`N` → `Patient.deceased`). PID-30 wins when present; otherwise the presence of PID-29 implies `deceased = true`. `pid_from_patient` emits both fields when the patient is flagged deceased, and otherwise keeps the segment compact at 14 fields (no wire-shape change for living patients).
- **A40 patient-merge** (v0.18): the inbound message carries the **survivor** in PID and the **source** MRN in MRG-1.1 (`<value>^^^<facility>^MR` shape, mirroring PID-3). Survivor PID runs the same dedup-or-create path as A01 / A28 (bootstrap-friendly: unknown survivors get created on the fly). Source MRN must resolve to an existing PAS row via `PatientRepository::find_by_identifier_value`; otherwise the handler returns `404 + AE`. The merge itself is the same single DB transaction as `POST /api/patients/{id}/merge-into/{target_id}`: `set_replaced_by(source_id → target_id)` + `audit_log` (`action = "merge_via_hl7v2_a40"`) + `outbox_events` (`event_type = "PatientMerged"`, payload `{source_id, target_id, source: "hl7v2_a40"}`). Post-commit the source is dropped from the Tantivy index best-effort. Conflict matrix: source already a tombstone → `409 + AE` (`already merged into …`); MRG-1 resolves to the same row as PID → `409 + AE` (self-merge); missing MRG / empty MRG-1.1 → `400 + AE`. Outbound mapping in `Hl7v2MllpPublisher`: `PatientMerged → ADT^A40` (loads both patients by id to populate PID + MRG), **skipped when payload `source == "hl7v2_a40"`** so we don't echo the merge back to the EMR that just sent it; REST-driven merges (no source tag) relay normally.
- **MFN^M02 Master File Notification — Staff** (v0.24): inbound `POST /api/hl7/v2/mfn-staff` accepts an MFN^M02 message carrying one MFI segment plus one-or-more MFE+STF pairs. Each pair is a master-file row event: `MAD` (add) creates a new `practitioners` row; `MUP` (update) full-replaces an existing row's name + gender + birth_date + active flag; `MDL` (delete) **soft-deletes** by flipping `active = false` (practitioner rows hold downstream encounter / appointment / schedule references and hard-delete would orphan them). The EMR's staff id (MFE-4 with STF-1 fallback) is stored on the practitioner row as `Identifier { type: Other, system: "urn:hl7v2:staff:id" }`; PAS uses `PractitionerRepository::find_by_identifier_value` (Postgres JSONB `@>`) to locate the row for `MUP` / `MDL`. STF subset honored: STF-3 (name, XPN: `family^given^middle`), STF-5 (`M`/`F`/`O`/`U` → `Gender`), STF-6 (`YYYYMMDD` → `birth_date`), STF-7 (`A`/`I` → `active`, defaults to `A`). MFI-1 must equal `"PRA"` (other master-file ids AE-ACK as unsupported); MFE-1 must equal `MAD` / `MUP` / `MDL`; pre-check matrix rejects with `409 + AE` (MAD on a known staff id) or `404 + AE` (MUP/MDL on an unknown staff id) before opening the DB transaction. All items are applied in **one DB transaction** — the v0.20 multi-FT1 contract reused: a duplicate-MAD in the second slot rolls back the first slot's MAD. Per-item audit (`action = "{mad|mup|mdl}_via_hl7v2_mfn_m02"`) and outbox (`PractitionerCreated` / `PractitionerUpdated` / `PractitionerDeactivated` with `source: "hl7v2_mfn_m02"`) are written inside the transaction. AA ACK MSA-3 reports `practitioner=<uuid> event=<...>` for single-item messages, `staff_records_applied=<N>` for multi-item. MLLP listener auto-routes MFN^M02 to `/api/hl7/v2/mfn-staff`.
- **DFT^P03 financial transaction** (v0.19 + v0.20): the inbound message carries one patient in PID and **one or more charges** in FT1 segments. Honored FT1 fields per segment: FT1-4 (`YYYYMMDDHHMMSS` → `Charge.posted_at`; defaults to "now"), FT1-6 (transaction type — `CG` only; `PY` / `AJ` AE-ACK as unsupported; empty defaults to `CG`), FT1-7.1 (transaction code → `Charge.code`, required), FT1-8 (description → `Charge.description`, required), FT1-11.1 (decimal amount), FT1-11.2 (ISO 4217 currency, 3 uppercase ASCII letters). The handler dedup-or-creates the patient from PID, finds an open billing account for the patient via `BillingRepository::find_open_account_for_patient`, **auto-creates one in the FT1-11.2 currency when none exists**, and posts every charge inside a single DB transaction via `BillingRepository::create_charge`. v0.20 adds the **multi-FT1 contract**: all FT1 segments in one message must share the same FT1-11.2 currency (mixing → `400 + AE`); the whole transaction is **all-or-nothing** so a single bad row rolls back every charge from the message. Writes audit (`action = "post_via_hl7v2_p03"`) and outbox (`event_type = "ChargePosted"`, payload `{charge_id, account_id, patient_id, source: "hl7v2_p03"}`) once per FT1, all inside the same transaction. Per-FT1 parse errors name the 1-based segment index (`FT1[2]-11.1 ...`). AA ACK reports `charge=<uuid> account=<uuid>` for the single-FT1 case (backwards-compatible with v0.19) and `charges_posted=<N> account=<uuid>` for multi-FT1 in MSA-3. Outbound mapping in `Hl7v2MllpPublisher`: one `DFT^P03` per `ChargePosted` outbox event — resolves `charge → account → patient` via the new `BillingRepository::{find_charge_by_id, find_account_by_id}`. **Skipped when payload `source == "hl7v2_p03"`** (boomerang protection); REST-driven charges (no source tag, from `BillingService::post_charge`) relay normally.
- **HL7 v2 SIU** (v0.16 + v0.17): four HTTP endpoints — `POST /api/hl7/v2/schedule-book` (SIU^S12), `/schedule-reschedule` (SIU^S13), `/schedule-modify` (SIU^S14), `/schedule-cancel` (SIU^S15). The MLLP listener auto-routes each trigger to the matching endpoint. Segment subset honored on parse: `SCH-1` (placer appointment id, opaque), `SCH-2` (filler — PAS UUID), `SCH-7` (reason → `Appointment.reason`), `SCH-9` / `SCH-10` (duration value + units; default 30 min when missing or non-numeric), `SCH-11` (start datetime — `YYYYMMDDHHMMSS` / `YYYYMMDDHHMM` / `YYYYMMDD`; trailing `±HHMM` offsets stripped and treated as UTC). **Required field invariants**: `S12` requires SCH-11; `S13` requires SCH-2 + SCH-11; `S14` and `S15` require SCH-2. Inbound S12 dedups patient on MRN, blocks overlapping appointments (409 + AE), writes audit (`book_via_hl7v2_s12`) + outbox (`AppointmentBooked` with `source: "hl7v2_s12"`); AA ACK's MSA-3 carries `filler=<uuid>`. Inbound S13 looks up by SCH-2, validates non-terminal status, runs an **overlap-excluding** check via `AppointmentRepository::find_overlapping_for_patient_excluding` (so the row's own current window doesn't flag itself), then updates `start_datetime` + `end_datetime`. Inbound S14 updates only `Appointment.reason` from SCH-7; time fields ignored. Inbound S15 cancels by filler id. S13/S14/S15 all return `404 + AE` for unknown filler, `409 + AE` for already-terminal status, `400 + AE` for missing or malformed SCH-2. Audit actions are `reschedule_via_hl7v2_s13` / `modify_via_hl7v2_s14` / `cancel_via_hl7v2_s15`; outbox event types `AppointmentRescheduled` / `AppointmentModified` / `AppointmentCancelled` carry `source: "hl7v2_s13"` / `"hl7v2_s14"` / `"hl7v2_s15"`. Outbound mapping in `Hl7v2MllpPublisher`: `AppointmentBooked → SIU^S12`, `AppointmentRescheduled → SIU^S13`, `AppointmentModified → SIU^S14`, `AppointmentCancelled → SIU^S15`, all **skip relaying when payload `source == "hl7v2_*"`** (boomerang protection). Unsupported triggers (S17/S26/…) fall through to `/api/hl7/v2/patient` and AE-ACK with `unsupported SIU trigger`.
- **Batch envelope** (v0.6): `POST /api/hl7/v2/batch` accepts an `FHS`/`BHS`/`BTS`/`FTS` wrapper containing up to `MAX_BATCH_MESSAGES = 1000` `MSH` messages. Each contained message is dispatched independently to the matching single-message handler (`hl7_v2_admit` / `_transfer` / `_discharge` / `_update` / `_cancel_admit` / `_cancel_discharge` / `_patient`); responses are stitched into one batch ACK envelope: `BHS … <per-message MSH+MSA blocks> … BTS|<n>`. **Per-message independence**: one failure does not roll back the others (matches real-world batch processors and mirrors the PAS FHIR `batch` Bundle path — the FHIR `transaction` Bundle is the all-or-nothing option). Per-message AA/AE/AR lives inside the envelope; outer HTTP status is always `200 OK` when the envelope itself parses. `400 + AR` returned on a malformed envelope, oversize batch, or multi-batch file (more than one `BHS` per transmission is unsupported). Bare `MSH` lists without a `BHS` envelope are accepted for convenience. The MLLP listener auto-routes any payload starting with `FHS` or `BHS` to the batch endpoint.
- **MLLP TCP listener** (opt-in via `HL7V2_MLLP_BIND`): consumes MLLP-framed (`\x0b<payload>\x1c\x0d`) messages; dispatches each frame to the matching HTTP route via `axum::Router::oneshot`. **Bearer auth does not apply on the MLLP path** — use network-level controls.
- **Outbound MLLP** (opt-in via `HL7V2_OUTBOUND_PEER`): `Hl7v2MllpPublisher` implements `EventPublisher`. Mapping: `EncounterAdmitted → ADT^A01`, `EncounterTransferred → ADT^A02`, `EncounterDischarged → ADT^A03`, `PatientUpdated → ADT^A08` (**only when `source: "hl7v2_a08"`** — REST-driven patient edits do not boomerang), `EncounterCancelled → ADT^A11` (**only when `reason: "hl7v2_a11"`**), `EncounterDischargeCancelled → ADT^A13`. Builds the corresponding ADT, opens a fresh TCP connection to the peer, writes the frame, reads the ACK. Returns `Ok(())` only on `MSA|AA`; anything else leaves the outbox row unpublished for retry. Non-mapped event types drop silently.

#### 4.8.4 OpenAPI

- `src/api/openapi.rs::ApiDoc` aggregates every `#[utoipa::path]` handler (**131 handlers across 19 tags** at v0.43.0; was 92 at v0.6.0, 91 at v0.5, 89 at v0.4, 86 at v0.3.4 — every ADT / SIU / DFT / MFN addition since brings a new annotated handler).
- Swagger UI at `/swagger-ui`; raw spec at `/api-docs/openapi.json`.

### 4.9 Observability & Ops

- **Audit query endpoints**: `GET /api/patients/:id/audit`, `GET /api/audit/recent?limit=N` (capped at 500), `GET /api/audit/entity?entity_type=…&entity_id=…&limit=N`.
- **Outbox diagnostics**: `GET /api/admin/outbox/unpublished?limit=N`.
- **Dead-letter queue** (v0.5): `GET /api/admin/outbox/dead-letters?limit=N` (newest-first, capped at 500), `POST /api/admin/outbox/dead-letters/{id}/replay` (atomically re-inserts the event back into `outbox_events` with `retry_count = 0` and deletes the DLQ row; returns `{ dead_letter_id, new_outbox_id }`).
- **Health**: `GET /api/health` pings the DB; returns `{ status: "ok"|"degraded", database: "ok"|"unreachable" }`. Always exempt from bearer auth.
- **Ops dashboard**: `GET /dashboard` Tera + HTMX page with four panels (ward occupancy, RTT breaches, outbox unpublished count, recent audit). Four fragment endpoints (`/dashboard/{wards,breaches,outbox,audit}`) refresh every 10 s via `hx-get`. First load works **without JavaScript** — every fragment is inlined; HTMX is a progressive enhancement. Markup uses Lily Design System headless class names (`header`, `footer`, `panel`, `data-table*`, `badge`, `alert`, `code`); ARIA tightened (`scope="col"`, `aria-label` on panels + tables).
- **Structured logging**: `tracing::Subscriber` with `EnvFilter` (driven by `RUST_LOG`) + a stdout fmt layer. Every `#[tracing::instrument]` span and `info!`/`warn!` call lands here.
- **Distributed tracing** (v0.7): when `OTLP_ENDPOINT` is set, `observability::init` additionally wires a `tracing_opentelemetry` layer backed by an OTLP HTTP/protobuf exporter (transport via `reqwest`; no `tonic`). Service name comes from `OTEL_SERVICE_NAME` (default `pas-axum`). Span export runs on the tokio runtime via a batch processor — never blocks the request path. Setup failures (malformed endpoint, exporter build error) log a `warn!` and fall back to fmt-only — the server still boots. When `OTLP_ENDPOINT` is unset, no network egress and no behavior difference from v0.6.
- **Per-IP rate limit** (v0.12): a hand-rolled token-bucket tower middleware caps incoming HTTP request rate per peer IP. Tuning via `PAS_RATE_LIMIT_RPM` (default `600`; sustained refill rate in req/min; `0` disables) and `PAS_RATE_LIMIT_BURST` (default `60`; bucket capacity). Layer sits **outside bearer auth** (so brute-force token guessing is throttled) and **inside trace** (so 429s still get logged). Peer IP comes from `axum::extract::ConnectInfo<SocketAddr>` — the PAS calls `into_make_service_with_connect_info::<SocketAddr>()` to surface it. **Not** `X-Forwarded-For`-aware (spoofable without an explicit trusted-proxy layer); deployments behind an L7 proxy that need true per-client limiting should run their own. **`/api/health` is exempt** so operational pings never trip the limiter. On cap: status `429`, header `Retry-After: <seconds>` rounded up, body in the standard `ApiResponse` envelope with `error.code = "RATE_LIMITED"`. Bucket cleanup runs when the map grows past 50 000 entries; entries idle >5 min are evicted.

---

## 5. Non-Functional Requirements / Invariants

These are **non-negotiable**. Every reviewer (human or agent) checks them.

| # | Invariant | Why |
|---|-----------|-----|
| 1 | **Identity and workforce are HTTP-delegated** to `person-service` and `worker-service`. The PAS schema holds no `patients` / `practitioners` / `practitioner_roles` / `departments` tables; PAS-owned rows reference identity via `person_id: Uuid` and `worker_id: Uuid` columns (no DB-level FK — the row lives in another service's database). | Single source of truth per concern; avoids dual identity stores. |
| 2 | **Hard runtime dependency** on both upstream services. PAS handlers that write a `person_id` or `worker_id` resolve the id against the upstream before persisting; unreachable upstream → `503 + UPSTREAM_UNAVAILABLE` envelope. No local read-through cache in v0.44.0 (see §4.0.1 decision callout). | Honest about coupling. Cache can be added in Phase 7 if latency or availability demand it. |
| 3 | Money is `rust_decimal::Decimal` + `Iso4217`. **Never** `f64`. | Financial precision; multi-currency safety. |
| 4 | Time at the domain layer is `chrono::DateTime<Utc>`. Naive local times never enter the domain. SeaORM `DateTimeWithTimeZone` at the DB; convert at the repository. | Audit determinism; timezone-safe arithmetic. |
| 5 | Soft-delete (`deleted_at TIMESTAMPTZ`) **only on** `encounters` and `appointments`. Other tables are append-only or operational. Patient/practitioner soft-delete is upstream's concern. | Audit retention vs. operational simplicity. |
| 6 | State changes touching multiple rows run in one `DatabaseTransaction` and write `audit_log` + `outbox_events` rows in the same transaction. Never split. **Cross-service writes (PAS + upstream) are *not* transactional** — see §4.8.2 for FHIR transaction Bundle implications. | Outbox invariant for PAS-owned writes; cross-service atomicity is given up by Option C. |
| 7 | `SELECT … FOR UPDATE` on contention points (bed allocation, slot booking). The partial unique index `bed_assignments (bed_id) WHERE released_at IS NULL` enforces "at most one active assignment per bed" at the database level. | Concurrent-booking safety. |
| 8 | PAS is administrative, not clinical. | Scope discipline. |
| 9 | `README.md` owns the full endpoint table. `AGENTS/restful.md` documents the shape but does not duplicate the list. | Single source of truth. |
| 10 | `patient-administration-system-frontend` never writes directly to the shared DB or to upstream services. Every write flow proxies through the PAS Axum REST API. | Audit + outbox completeness regardless of which app initiated the change. |
| 11 | Stateless server: every request reads from the DB (or upstream). Horizontal scaling by adding replicas behind a load balancer. | Availability. |
| 12 | The outbox dispatcher runs in-process. If multiple replicas run, each polls; duplicate delivery is possible. Production deployments wanting exactly-once should pin the dispatcher to one replica or wrap publish in an idempotent consumer. | Honest about delivery semantics. |
| 13 | All response envelopes for `/api/*` use `{ success, data, error }`. All `/fhir/*` responses return canonical FHIR resources on success and FHIR `OperationOutcome` on failure. | Predictable client contract. |
| 14 | `auth_test.rs` is the only DB-free integration test. Every other integration test calls `common::database_url()` first and skips silently when unset, so CI without Postgres remains green. Integration tests additionally use an embedded stub of the two upstream services (HTTP-server-on-localhost-via-`TcpListener`, same pattern as v0.14 `webhook_test.rs`). | Tests stay runnable without infra. |

---

## 6. Architecture

### 6.1 Workspace layout

```
.
├── Cargo.toml                                 workspace manifest (resolver 2) — three members + hoisted [profile.release]/[profile.bench]
├── CHANGELOG.md                               workspace-level changelog
├── README.md                                  workspace README
├── patient-administration-system-rust-crate/  PAS Axum API server (v0.44.0)
│   ├── Cargo.toml
│   ├── src/                                   library + bins
│   ├── tests/                                 19 integration test files
│   ├── benches/                               3 Criterion suites
│   ├── examples/                              cargo example + sample payloads
│   ├── migrations/                            sea-orm migrations crate
│   ├── templates/                             Tera dashboard templates (baked via include_str!)
│   ├── AGENTS.md + AGENTS/                    architecture / models / restful / matching / testing / interchange docs
│   ├── README.md + CHANGELOG.md
│   └── spec.md                                this file
└── patient-administration-system-frontend/                              Loco-rs front-end (v0.1.0)
    ├── Cargo.toml
    ├── src/                                   incl. csrf.rs middleware
    ├── assets/views/                          Tera + Lily + HTMX
    ├── config/                                Loco development/production/test YAML
    ├── tests/                                 dashboard_smoke.rs (13 tests)
    ├── README.md + CHANGELOG.md
```

Workspace-level profile blocks (`[profile.release]` LTO + single codegen unit + strip; `[profile.bench]` inherits release) live in the root `Cargo.toml` because per-member profile blocks are silently ignored by Cargo. One shared `Cargo.lock`, one shared `target/`.

### 6.2 Layered design (PAS Axum)

Strict five-layer service: `api → service → (repository → db) | (upstream → person-service / worker-service)`.

- **api** (`src/api/`) — Axum routers for REST and FHIR, `#[utoipa::path]` annotations, dashboard handler. Handlers extract user context from `X-User-Id` / `X-User-Ip` / `X-User-Agent`, decode + validate the JSON body, call a service, serialize the response into `ApiResponse<T>`. **No business logic in handlers.**
- **service** (`src/{adt, scheduling, waitlist, resources, billing, communication}/`) — one module per aggregate cluster. Owns business rules, state-machine transitions, transactional boundaries. Services receive both `&DatabaseConnection` and `&dyn PersonClient` / `&dyn WorkerClient` by injection; resolving a `person_id` is just an upstream call.
- **repository** (`src/db/repositories/`) — one repo per **PAS-owned** aggregate (encounter, admission, appointment, bed, schedule, slot, waitlist, rtt, letter, billing, consent, audit, outbox). Talks to sea-orm. The `db::with_txn(db, |txn| async { … })` helper composes multiple repos inside one transaction. `AuditLogRepository::log` and `OutboxRepository::publish` accept any `&C: ConnectionTrait`, composing with primary writes in the same transaction. **No `PatientRepository` or `PractitionerRepository`** — those identities live upstream.
- **db** (`src/db/`) — `connect(url)` (pool 2–10, 8 s connect timeout, 60 s idle timeout, `sqlx_logging` off), `with_txn`, ~28 SeaORM entities (`src/db/entities/*.rs`, one file per table). Down from 32 at v0.43.0 because `patient`, `practitioner`, `practitioner_role`, `department` are gone.
- **upstream** (`src/upstream/`) — `PersonClient` and `WorkerClient` traits plus reqwest-backed `HttpPersonClient` / `HttpWorkerClient` impls. Each client exposes the full upstream surface (`create`, `get`, `update`, `delete`, `search`, `match`, `check_duplicates`, `merge`, `masked`, `export`, `audit`). Stub implementations (`StubPersonClient`, `StubWorkerClient`) live alongside for integration tests, mirroring the v0.14 `webhook_test.rs` `TcpListener` pattern.

### 6.3 Transactional outbox

Every state-changing API call writes **three rows in one DB transaction**:

1. The entity row (insert or update).
2. An `audit_log` row recording `(entity_type, entity_id, action, old_value JSONB, new_value JSONB, user_id, user_ip, user_agent, at)`.
3. An `outbox_events` row carrying the domain-event payload.

Either all three land or none do.

A background dispatcher (`src/streaming/dispatcher.rs`, spawned in `main.rs` at startup) polls `outbox_events WHERE published = false` every 2 s, batches up to 64 rows, forwards them to the configured `EventPublisher`, and marks them `published = true`. The partial index `outbox_events (published) WHERE published = false` keeps the hot path cheap.

**Publisher selection (v0.14).** `main.rs` builds a chain at startup: each of `HL7V2_OUTBOUND_PEER` (HL7 v2 MLLP outbound, v0.4) and `PAS_WEBHOOK_URL` (HTTP webhook, v0.14), when set, contributes one child publisher. The chain is collapsed via `streaming::CompositePublisher` when ≥ 2 children are configured (sequential fan-out, first-failure-wins — outbox row stays unpublished and retries on the next tick if any child errors). With exactly one child the publisher is used directly (no composite overhead). With none, `InMemoryEventPublisher` takes over.

**Webhook publisher (v0.14).** `streaming::WebhookEventPublisher` POSTs each `DomainEvent` to `PAS_WEBHOOK_URL` as `application/json`. Always sends `X-PAS-Event-Id: <uuid>` and `X-PAS-Event-Type: <e.g. EncounterAdmitted>`. When `PAS_WEBHOOK_SECRET` is set, also sends `X-PAS-Signature: sha256=<lowercase hex>` — HMAC-SHA256 over the raw body, keyed by the secret. Receivers MUST verify it constant-time. 2xx ⇒ published; anything else (4xx, 5xx, DNS / TLS / connect / read / timeout) ⇒ `Error::Streaming`, outbox row stays pending, dispatcher retries. Receivers MUST be idempotent on `X-PAS-Event-Id`.

**Dead-letter queue (v0.5).** Failed publishes increment `outbox_events.retry_count`, record the diagnostic in `last_error`, and stamp `last_attempted_at`. After `PAS_OUTBOX_MAX_RETRIES` (default `10`) consecutive failures the dispatcher moves the row into `outbox_dead_letters` in one DB transaction (insert dead-letter + delete original). Operators inspect via `GET /api/admin/outbox/dead-letters` and replay via `POST /api/admin/outbox/dead-letters/{id}/replay`, which re-inserts the row into `outbox_events` with `retry_count = 0`. Rows in the DLQ are kept indefinitely; nothing else deletes them. Set `PAS_OUTBOX_MAX_RETRIES=0` to disable dead-lettering (retry forever). Dead-lettering applies uniformly to all publisher backends (HL7 v2 MLLP and webhook).

Domain event payloads cover **PAS-owned state changes only**: `EncounterAdmitted/Transferred/Discharged/Cancelled`, `EncounterDischargeCancelled`, `AppointmentBooked/Cancelled/CheckedIn/Completed/NoShow`, `WaitlistAdded/Removed`, `RTTClockStarted/Paused/Resumed/Stopped`, `BedStatusChanged`, `LetterGenerated`, `ChargePosted`, `InvoiceFinalized`, `PaymentPosted`.

> **REMOVED in v0.44.0:** `PatientCreated/Updated/Deleted/Merged` and `PractitionerCreated/Updated/Deactivated` — those events now fire from `person-service` / `worker-service` directly. PAS subscribers that need them subscribe to the upstream services. The PAS *consumes* merge notifications via the inbound webhook described in §2.3.

### 6.4 State machines (four)

Each implemented as an enum with `try_transition_to(self, next) -> Result<Self>` returning `Error::InvalidStateTransition` on any illegal move (same-state included).

**Encounter status**

```
Planned    -> Arrived | Cancelled
Arrived    -> InProgress | Cancelled
InProgress -> OnLeave | Finished | Cancelled
OnLeave    -> InProgress | Finished | Cancelled
Finished   -> (terminal)
Cancelled  -> (terminal)
```

**Appointment status**

```
Proposed  -> Booked | Cancelled
Booked    -> Arrived | Cancelled | NoShow
Arrived   -> Fulfilled | Cancelled
Fulfilled -> (terminal)
Cancelled -> (terminal)
NoShow    -> (terminal)
```

**Slot status**

```
Free       -> Busy | BlockedOut
Busy       -> Free
BlockedOut -> Free
```

Same-state transitions and `Busy ↔ BlockedOut` are rejected.

**Bed status**

```
Available    -> Occupied | Reserved | OutOfService
Occupied     -> Cleaning | OutOfService
Cleaning     -> Available | OutOfService
Reserved     -> Occupied | Available | OutOfService
OutOfService -> Available
```

Beds cannot go directly from `Occupied` to `Available` — they must pass through `Cleaning`. This is a deliberate invariant of the ADT flow: discharge releases a bed to `Cleaning`, not to `Available`.

The RTT clock has its own lifecycle (`Active` → `Paused` → `Active` → `Stopped`) but is realized as an append-only event log (`RTTClockEvent`), not an in-place enum transition.

### 6.5 Aggregates

| Aggregate            | Root                  | Members                                                       |
| -------------------- | --------------------- | ------------------------------------------------------------- |
| *Patient*            | *(delegated)*         | Owned by `person-service`; PAS references via `person_id: Uuid`. |
| *Workforce*          | *(delegated)*         | Owned by `worker-service`; PAS references via `worker_id: Uuid`. |
| Facility             | `Facility`            | `Ward`, `Room`, `Bed` (each its own table; loaded by id)      |
| Encounter            | `Encounter`           | (state-machine root for inpatient/outpatient/ED)              |
| Admission            | `Encounter`           | `Admission`, `Transfer`, `Discharge`, `BedAssignment`         |
| Schedule             | `Schedule`            | `Slot` (one Schedule has many Slots)                          |
| Appointment          | `Appointment`         | optional `Slot` link, optional waitlist provenance, optional `series_id` backlink |
| Appointment series   | `AppointmentSeries`   | `RecurrenceRule`, N generated `Appointment` rows               |
| Waitlist             | `WaitlistEntry`       | optional originating `Referral`                               |
| RTT                  | `RTTPathway`          | append-only `RTTClockEvent` log                               |
| Letter               | `LetterTemplate`      | `GeneratedLetter` per render                                  |
| Billing              | `Account`             | `Charge`, `Invoice`, `Payment`                                |
| Coverage             | `Coverage`            | one record per `person_id` × policy; optional `account_id` link |
| Consent              | `Consent`             | one record per `person_id` × `ConsentType`                    |

### 6.6 Module map (PAS Axum)

```
src/
├── lib.rs                     Public re-exports
├── main.rs                    Binary entry (Axum + optional MLLP listener + outbox dispatcher + upstream client builders)
├── error.rs                   Error enum (12 variants — adds `Upstream`), Result alias
├── config/                    Env loader (incl. PERSON_SERVICE_URL, WORKER_SERVICE_URL)
├── models/                    Pure-Rust domain types for PAS-owned aggregates (no Patient/Practitioner here)
├── db/
│   ├── mod.rs                 connect(), with_txn()
│   ├── entities/              ~28 SeaORM entities (no patient/practitioner/practitioner_role/department)
│   └── repositories/          One per PAS-owned aggregate + audit.rs + outbox.rs
├── upstream/                  HTTP clients for person-service + worker-service
│   ├── mod.rs                 PersonClient, WorkerClient traits
│   ├── person_client.rs  reqwest-backed impl + StubPersonClient (for tests)
│   ├── worker_client.rs  reqwest-backed impl + StubWorkerClient (for tests)
│   ├── translate.rs           Patient ↔ Person, Practitioner ↔ Worker shape mapping
│   └── webhook.rs             inbound merge-notification handler
├── adt/                       Admit / transfer / discharge service (resolves person_id + worker_id upstream)
├── scheduling/                Slot booking + overlap detection (resolves person_id + worker_id upstream)
├── waitlist/                  Priority queue + RTT clock arithmetic
├── resources/                 Bed status + ward occupancy
├── billing/                   Charge / invoice / payment service
├── communication/             Tera letter rendering
├── hl7v2/                     Parser, encoder, mapping, ACK, escapes, MLLP framing + listener
├── interchange/               JSON / XML / TSV / CSV bulk patient surface — every row proxies upstream
├── api/
│   ├── mod.rs                 ApiResponse, ApiError, AppState (carries the two upstream clients)
│   ├── rest/                  Axum router + handlers + auth middleware
│   ├── fhir/                  FHIR R5 converters + handlers + OperationOutcome + Bundle write
│   ├── dashboard.rs           Ops dashboard (Tera + HTMX, Lily markup)
│   └── openapi.rs             ApiDoc aggregator
├── streaming/                 EventPublisher trait + InMemory + Hl7v2MllpPublisher + WebhookEventPublisher + CompositePublisher fan-out + dispatcher
├── validation/                Phone, address, dates, charge amount, RTT order (no patient identifier validators — upstream owns)
├── privacy/                   Masking, GDPR export aggregator (delegates demographics to upstream, joins PAS-owned related rows), consent gate
├── observability/             tracing-subscriber init
└── bin/seed.rs                pas-seed binary (demo data — calls person-service / worker-service to provision identities)
```

### 6.7 Error handling

A single `Error` enum in `src/error.rs` covers **12 variants** (gains `Upstream` in v0.44.0). The api layer converts errors to HTTP status codes:

| Variant                  | REST status | REST code                  | FHIR status        |
| ------------------------ | ----------- | -------------------------- | ------------------ |
| `NotFound`               |         404 | `NOT_FOUND`                | 404 (`not-found`)  |
| `Validation`             |         400 | `VALIDATION`               | 400 (`invalid`)    |
| `Fhir`                   |         400 | `FHIR`                     | 400 (`invalid`)    |
| `Conflict`               |         409 | `CONFLICT`                 | 409 (`conflict`)   |
| `InvalidStateTransition` |         409 | `INVALID_STATE_TRANSITION` | 409 (`conflict`)   |
| `Database`               |         500 | `DATABASE`                 | 500 (`exception`)  |
| `Search`                 |         500 | `SEARCH`                   | 500 (`exception`)  |
| `Streaming`              |         500 | `STREAMING`                | 500 (`exception`)  |
| `Render`                 |         500 | `RENDER`                   | 500 (`exception`)  |
| `Config`                 |         500 | `CONFIG`                   | 500 (`exception`)  |
| `Upstream`               |         503 | `UPSTREAM_UNAVAILABLE`     | 503 (`transient`)  |
| `Internal`               |         500 | `INTERNAL`                 | 500 (`exception`)  |

`Error::Upstream` is raised by the `upstream` layer on transport / TLS / DNS / read failures *and* on non-2xx responses from `person-service` / `worker-service` that do not carry an envelope the PAS can translate. Upstream 4xx responses that *do* carry a standard envelope are translated to the matching PAS variant (`UPSTREAM 404 → NotFound`, `400 + invalid → Validation`, etc.).

Success responses use `200 OK` (or `204 No Content` for `DELETE /fhir/Patient/:id`).

### 6.8 Operational shape under Option C

- **Three services in the request path** for any patient/practitioner write: PAS → person-service → upstream DB; or PAS → worker-service → upstream DB.
- **No local cache.** Every request re-resolves identities. Latency-sensitive flows (HL7 v2 ADT^A01 ingest at high rates) should run the upstream services co-located (same pod / same VLAN). A future 1-minute LRU on `GET /api/persons/{id}` is the Phase 7 escape hatch.
- **Liveness probe extension.** `GET /api/health` returns `{ database, person_service, worker_service }`. Any one of the three `unreachable` flips overall status to `degraded` (still 200; downstream load balancers should not yank a degraded replica out of rotation purely on this — make that call at the deployment layer).
- **Auth pass-through.** PAS handlers attach `X-User-Id` / `X-User-Ip` / `X-User-Agent` from the inbound request to every upstream call, so upstream audit rows credit the original caller. PAS also attaches `Authorization: Bearer <PERSON_SERVICE_TOKEN>` (configured at startup; static rotation, no per-request token).

---

## 7. Domain Model Specification

Full field-level reference: [`AGENTS/models.md`](AGENTS/models.md). Headline elements only here.

### 7.1 Shared value types (`models/mod.rs`)

- `Gender` — `Male` / `Female` / `Other` / `Unknown`. Serialized lowercase.
- `NameUse` — FHIR-aligned: `Usual`, `Official`, `Temp`, `Nickname`, `Anonymous`, `Old`, `Maiden`.
- `AddressUse` — `Home`, `Work`, `Temp`, `Old`, `Billing`.
- `Address` — line1, line2, city, state, postal_code, country (all optional).
- `ContactPointSystem` — `Phone`, `Fax`, `Email`, `Pager`, `Url`, `Sms`, `Other`.
- `ContactPointUse` — `Home`, `Work`, `Temp`, `Old`, `Mobile`.
- `ContactPoint` — system, value, use_type.
- `Iso4217` — newtype `String`; validates exactly three uppercase ASCII letters at construction.
- `Money` — `{amount: Decimal, currency: Iso4217}`. Methods: `new`, `zero`, `try_add` (returns `Error::Validation` on currency mismatch), `impl Add` (panics on mismatch — use `try_add` outside known-safe sites).
- `TimeRange` — `{start: DateTime<Utc>, end: DateTime<Utc>}`. Methods: `is_valid()`, `overlaps(&other)` (half-open; touching ranges do not overlap).

### 7.2 PAS-owned aggregates: resources, ADT, scheduling, waitlist, RTT, communication, billing, consent, coverage

See [`AGENTS/models.md`](AGENTS/models.md) for each `models/*.rs` file's full field list, factory methods, and state-machine signatures. Headline contracts:

- `BedStatus::try_transition_to`, `EncounterStatus::try_transition_to`, `SlotStatus::try_transition_to`, `AppointmentStatus::try_transition_to` — state-machine guards.
- `compute_active_weeks(events, now) -> u32` — RTT clock arithmetic (free function in `models::rtt`).
- `Invoice::finalize(&mut self) -> Result<()>` — only valid from `Draft`.
- `Consent::is_active(today: NaiveDate) -> bool` — active on the expiry day itself.
- `Encounter`, `Admission`, `Transfer`, `Discharge`, `BedAssignment`, `Appointment`, `AppointmentSeries`, `WaitlistEntry`, `RTTPathway`, `RTTClockEvent`, `LetterTemplate`, `GeneratedLetter`, `Account`, `Charge`, `Invoice`, `Payment`, `Coverage`, `Consent`, `Facility`, `Ward`, `Room`, `Bed` — all unchanged shape-wise, except every former `patient_id: Uuid` field is now `person_id: Uuid` and every former `practitioner_id: Uuid` is now `worker_id: Uuid`.

### 7.3 Delegated aggregates

`Patient` and `Practitioner` (and their members `PractitionerRole`, `Department`) are **not** PAS domain types in v0.44.0. When a handler needs demographic detail it calls the upstream service via the `upstream` layer and works with the upstream's `Person` / `Worker` shape directly (deserialized into types defined in `src/upstream/translate.rs`). The wire shape the PAS exposes to its own callers (`Patient` / `Practitioner`) is built by translating from the upstream shape inside the handler — see §4.0 + §4.0.1 for the rule.

---

## 8. API Specification

### 8.1 REST envelope

All `/api/*` responses use:

```json
{ "success": true,  "data": { ... }, "error": null }
{ "success": false, "data": null,    "error": { "code": "...", "message": "..." } }
```

FHIR endpoints return canonical FHIR resources on success and FHIR `OperationOutcome` on failure.

### 8.2 Authentication & user context

- When `API_TOKEN` is set, every `/api/*` request **except `/api/health`** must carry `Authorization: Bearer <token>` matching the configured value. Mismatched or missing tokens return `401 + ApiResponse::err("UNAUTHORIZED", ...)`. The bearer middleware is closer to the handlers than CORS/trace/compress, so a 401 still has the response middleware applied.
- When `API_TOKEN` is unset, the API runs in **trusted-caller mode** with a startup `warn!` log. Full JWT/OAuth identity-provider integration is out of scope.
- User context for the audit log is taken from headers on every state-changing request: `X-User-Id`, `X-User-Ip`, `X-User-Agent` (all optional, recorded as-is).
- **Bearer auth does not apply on the MLLP path.** Gate MLLP traffic at the network layer (private VLAN, firewall, mTLS via a sidecar).

### 8.3 CORS

Configurable via `CORS_ORIGINS` env var (comma-separated allowlist). Empty/unset = permissive with a startup warn; otherwise restricts to the listed origins.

### 8.4 Full route list

Lives in [`README.md`](README.md#api-endpoint-reference) — the single source of truth for endpoints. Approximately 85 routes across 19 OpenAPI tags. [`AGENTS/restful.md`](AGENTS/restful.md) documents the response envelope, error mapping, and headline request shapes; it does not duplicate the route list.

### 8.5 FHIR R5 — see §4.8.2.

### 8.6 HL7 v2 ADT — see §4.8.3.

### 8.7 Bulk interchange — see §4.8.1.

### 8.8 Dashboard — see §4.9.

---

## 9. Data Model Specification

### 9.1 Schema overview

**~28 tables in PostgreSQL** at v0.44.0 (down from 32 at v0.43.0), grouped:

| Group           | Count | Tables                                                                 |
|-----------------|-------|------------------------------------------------------------------------|
| *Identity*      | *0*   | *(delegated to `person-service`)*                                 |
| *Workforce*     | *0*   | *(delegated to `worker-service`)*                                 |
| Resources       | 4     | `facilities`, `wards`, `rooms`, `beds`                                 |
| ADT             | 5     | `encounters`, `admissions`, `transfers`, `discharges`, `bed_assignments` |
| Scheduling      | 4     | `schedules`, `slots`, `appointments`, `appointment_series`             |
| Waitlist + RTT  | 4     | `referrals`, `waitlist_entries`, `rtt_pathways`, `rtt_clock_events`    |
| Communication   | 2     | `letter_templates`, `generated_letters`                                |
| Billing         | 5     | `accounts`, `charges`, `invoices`, `payments`, `coverages`             |
| Consent         | 1     | `consents`                                                             |
| Audit + Events  | 3     | `audit_log`, `outbox_events`, `outbox_dead_letters`                    |

Every former `patient_id UUID` column on a PAS-owned table is now `person_id UUID`. Every former `practitioner_id UUID` is now `worker_id UUID`. These columns carry **no DB-level FK constraint** because the referenced row lives in another service's database.

### 9.2 Strategic indexes

| Index                                                       | Purpose                                |
|-------------------------------------------------------------|----------------------------------------|
| `appointments (person_id, start_datetime)`                  | Per-person overlap check               |
| `slots (schedule_id, start_datetime) WHERE status = 'Free'` | Slot search                            |
| `bed_assignments (bed_id) WHERE released_at IS NULL`        | At most one active per bed (partial UNIQUE) |
| `waitlist_entries (target_service, priority, created_at)`   | Priority queue scan                    |
| `rtt_clock_events (pathway_id, event_at)`                   | Clock arithmetic ordering              |
| `outbox_events (published) WHERE published = false`         | Dispatcher hot path                    |

Authoritative source: the original five migrations (`m20260520_000001_init`, `m20260525_000002_outbox_dlq`, `m20260526_000003_appointment_series`, `m20260527_000004_coverage`, `m20260528_000005_patient_replaced_by`) plus the v0.44.0 forward-only migration `m20260601_000006_delegate_identity` that:

1. Drops `patients`, `practitioners`, `practitioner_roles`, `departments` (and every index touching them, including the v0.11 `replaced_by` partial index).
2. Renames `patient_id → person_id` on 18 dependent columns across 9 tables (accounts, appointments, consents, encounters, generated_letters, referrals, rtt_pathways, waitlist_entries, plus `coverages.patient_id`).
3. Renames `practitioner_id → worker_id` on 13 dependent columns across 5 tables (admissions, appointments, encounters, schedules, transfers/discharges where applicable).
4. Drops the local Tantivy patient index directory.

The migration is **forward-only** (no `down` implementation). See §13 Layer 11 + risk #2 below.

### 9.3 Time handling

- Domain layer: `chrono::DateTime<Utc>` throughout.
- DB layer: SeaORM `DateTimeWithTimeZone` (a `chrono::DateTime<FixedOffset>`) for columns; conversion to/from `Utc` happens at the repository boundary.
- Effective time vs system time: every entity has `created_at` / `updated_at` for system bookkeeping. Entities with their own event time additionally carry domain-time columns (`Encounter.period_start/end`, `Admission.admitted_at`, `Transfer.transferred_at`, `Discharge.discharged_at`, `Slot.start_datetime/end_datetime`, `Appointment.start_datetime/end_datetime`, `RTTClockEvent.event_at`). A corrective backdated entry can move `event_at` without altering `created_at`.

---

## 10. Dependency Stack Specification

Mirrors `Cargo.toml`. The v0.3.1 audit removed 15 unused direct deps; v0.3.4 removed 2 unused dev-deps. Anything below is actually compiled.

| Component            | Crate                                                                  |
|----------------------|------------------------------------------------------------------------|
| Async runtime        | `tokio` (`full`)                                                       |
| Web                  | `axum` 0.7 (`macros`)                                                  |
| HTTP middleware      | `tower` (`util`), `tower-http` (`cors`, `trace`, `compression-full`)   |
| Templates            | `tera` 1.20                                                            |
| ORM                  | `sea-orm` 1.1 (`sqlx-postgres`, `runtime-tokio-rustls`, `with-chrono`, `with-uuid`, `with-json`) |
| Migrations           | `sea-orm-migration` 1.1                                                |
| Upstream HTTP client | `reqwest` 0.12 (`json`, `rustls-tls`) — promoted from transitive to direct in v0.44.0 for the two `PersonClient` / `WorkerClient` impls; was already a direct dep at v0.14 for `WebhookEventPublisher`. |
| OpenAPI              | `utoipa` 5.4 + `utoipa-swagger-ui` 8.1                                 |
| Serialization        | `serde`, `serde_json`, `quick-xml` 0.36 (`serialize`), `csv` 1.3       |
| Logging              | `tracing` 0.1, `tracing-subscriber` 0.3 (`env-filter`, `json`)         |
| Tracing exporter (v0.7) | `opentelemetry` 0.27, `opentelemetry_sdk` 0.27 (`rt-tokio`), `opentelemetry-otlp` 0.27 (`http-proto`, `reqwest-client`), `tracing-opentelemetry` 0.28 |
| IDs / time           | `uuid` 1.19 (`v4`, `serde`), `chrono` 0.4 (`serde`)                    |
| Env                  | `dotenvy` 0.15                                                         |
| Async traits         | `async-trait` 0.1                                                      |
| Errors               | `thiserror` 2.0                                                        |
| Money                | `rust_decimal` 1.36 (`serde-with-str`)                                 |
| Dev-deps             | `assertables` 9.8, `tempfile` 3.24, `criterion` 0.5                    |

**Removed in v0.44.0** (and why):

- `tantivy` 0.22 — local patient search index is gone; `person-service` owns full-text + fuzzy + phonetic search.

**Explicitly not included** (and why):

- **`person-service` / `worker-service` as Cargo deps.** Integration is **HTTP-only** (see §1 + §6.2). Adding the upstream crates as path / git deps would pull their full transitive trees (loco-rs, tonic, prost, openapiv3, fluvio, …) into this binary for no benefit; the wire shape is the contract.
- `mockall` / `tokio-test` — no `use mockall` or `tokio_test::` anywhere; `#[tokio::test]` comes from `tokio` itself.
- `loco-rs`, `fluvio`, `tonic` / `prost` / `tonic-build`, `openapiv3` — nothing uses them.
- `jsonwebtoken`, `argon2` — bearer-token middleware is the only auth in v0.1.
- `strsim` — fuzzy matching belongs to `person-service`.
- `anyhow`, `bigdecimal`, `validator`, `hyper` — `thiserror` + `rust_decimal` + hand-written validators + axum are sufficient.

---

## 11. Configuration Specification

Read from environment (or `.env`).

### 11.1 PAS Axum

| Variable               | Required | Default          | Purpose                                                  |
|------------------------|----------|------------------|----------------------------------------------------------|
| `DATABASE_URL`         | Yes      | —                | PostgreSQL connection string                             |
| `SERVER_HOST`          | No       | `0.0.0.0`        | HTTP bind address                                        |
| `SERVER_PORT`          | No       | `8080`           | HTTP port                                                |
| `PERSON_SERVICE_URL` | **Yes** | `http://localhost:5180` (dev only) | Base URL of the upstream `person-service`. **Required at startup in non-test mode**; PAS refuses to boot if unreachable on first ping. v0.44.0. |
| `PERSON_SERVICE_TOKEN` | No  | unset            | Bearer token attached to every outbound call to `person-service` via `Authorization: Bearer <token>`. v0.44.0. |
| `PERSON_SERVICE_WEBHOOK_SECRET` | No | unset       | HMAC-SHA256 secret used to verify inbound `POST /api/upstream/person-merged` calls. When unset, that endpoint refuses every request (`401 + UNSIGNED_WEBHOOK`). v0.44.0. |
| `WORKER_SERVICE_URL` | **Yes** | `http://localhost:5181` (dev only) | Base URL of the upstream `worker-service`. Same boot-time check as `PERSON_SERVICE_URL`. v0.44.0. |
| `WORKER_SERVICE_TOKEN` | No  | unset            | Bearer token attached to every outbound call to `worker-service`. v0.44.0. |
| `WORKER_SERVICE_WEBHOOK_SECRET` | No | unset       | HMAC-SHA256 secret for inbound `POST /api/upstream/worker-merged`. Same semantics as the person variant. v0.44.0. |
| `UPSTREAM_TIMEOUT_SECS` | No      | `10`             | reqwest timeout applied to every outbound call to either upstream service. v0.44.0. |
| `RUST_LOG`             | No       | `info`           | Tracing filter                                           |
| `API_TOKEN`            | No       | unset            | When set, bearer-auth enforced on `/api/*` except `/api/health` |
| `CORS_ORIGINS`         | No       | unset            | Comma-separated allowlist; unset = permissive            |
| `HL7V2_MLLP_BIND`      | No       | unset            | `host:port` for the MLLP TCP listener                    |
| `HL7V2_OUTBOUND_PEER`  | No       | unset            | `host:port` of a downstream MLLP receiver                |
| `PAS_OUTBOX_MAX_RETRIES` | No     | `10`             | Per-event retry budget for the outbox dispatcher (v0.5). After N consecutive failures the row is moved to `outbox_dead_letters`. Set `0` to disable dead-lettering. |
| `PAS_WEBHOOK_URL`      | No       | unset (disabled) | v0.14 — outbox webhook destination URL. When set, `WebhookEventPublisher` POSTs every event as JSON. Composes with `HL7V2_OUTBOUND_PEER` via `CompositePublisher` when both are set. |
| `PAS_WEBHOOK_SECRET`   | No       | unset            | v0.14 — HMAC-SHA256 secret for `X-PAS-Signature` header. |
| `PAS_WEBHOOK_TIMEOUT_SECS` | No   | `10`             | v0.14 — webhook request timeout. Ignored when `PAS_WEBHOOK_URL` is unset. |
| `OTLP_ENDPOINT`        | No       | unset            | OTLP collector endpoint (HTTP/protobuf, e.g. `http://otel-collector:4318/v1/traces`). When set, `observability::init` wires a `tracing_opentelemetry` layer alongside fmt; every span is exported. When unset, fmt only — no network egress. Setup failures are non-fatal. *(Wired in v0.7.)* |
| `OTEL_SERVICE_NAME`    | No       | `pas-axum`       | `service.name` resource attribute on every exported span. Override per replica / environment. *(Added in v0.7.)* |
| `PAS_SMS_PROVIDER`     | No       | `none`           | SMS provider selector (v0.8). `none` = `NoopSmsProvider` (auto-send disabled, SMS letters stay `Pending` — same as v0.7). `log` = `LogSmsProvider` (every outbound message logged at `tracing::info!(target: "pas::sms")`, letter flipped to `Sent`). Unknown values fall back to `none` with a startup `warn!`. Case- and whitespace-insensitive. |
| `PAS_RATE_LIMIT_RPM`   | No       | `600`            | Per-IP rate-limit (v0.12) sustained refill rate, in requests per minute. `0` disables the layer entirely. |
| `PAS_RATE_LIMIT_BURST` | No       | `60`             | Per-IP rate-limit bucket capacity. Burst tolerance before the limiter throttles a client to the sustained refill rate. Ignored when `PAS_RATE_LIMIT_RPM=0`. |

### 11.2 patient-administration-system-frontend

| Variable              | Default                  | Purpose                                                              |
|-----------------------|--------------------------|----------------------------------------------------------------------|
| `DATABASE_URL`        | from `config/*.yaml`     | PostgreSQL connection string (same DB as PAS Axum)                   |
| `PAS_API_URL`         | `http://localhost:8080`  | Base URL for the PAS Axum API — every write flow routes through it   |
| `PAS_COOKIE_SECURE`   | unset                    | Set to `1`/`true`/`yes` so the `pas_csrf` cookie carries `Secure` (HTTPS-only). Off in dev. |

---

## 12. Testing & Quality Specification

### 12.1 Coverage targets

**Shipped baseline (v0.44.0, as of 2026-06-01)** — what the code currently passes:

| Suite                                                   | Count   | Notes                                              |
|---------------------------------------------------------|---------|----------------------------------------------------|
| PAS lib unit tests                                      | 358     | `cargo test --lib`, all green, ~3 s                |
| `patient-administration-system-frontend` lib tests      | 17      | csrf (9) + person_api (3) + patient view-model (5) |
| PAS integration test files                              | 27      | DB-gated; net of v0.43 minus `patient_crud_test.rs` / `patient_merge_test.rs`, plus `upstream_person_proxy_test.rs` + `upstream_worker_proxy_test.rs` + `upstream_merge_webhook_test.rs` |
| `patient-administration-system-frontend` integration tests | 13   | dashboard smoke + form CSRF + 404 + health         |
| Criterion benchmark suites                              | 3       | adt, scheduling, waitlist                          |

**Historical baseline (v0.43.0)** — pre-Option-C, kept here for delta-tracking only:

| Suite                                                   | Count   | Delta (v0.44 − v0.43) | Notes |
|---------------------------------------------------------|---------|-----------------------|-------|
| PAS lib unit tests                                      | 497     | −139                  | Patient / Practitioner / Identifier model + validator + Tantivy search unit tests removed; new `upstream::` + `hl7v2::encode` + `streaming::hl7v2_publisher` + FHIR write tests added |
| PAS integration test files                              | 24      | +3                    | `patient_*` removed; `upstream_*` (3 files) added |

### 12.2 Quality gates

Run from the workspace root (the workspace `Cargo.toml` lists all three members):

```bash
# v0.44.0 (shipped today):
cargo build  --workspace --all-targets             # zero errors, zero warnings
cargo test   --workspace --lib                     # 358 PAS + 17 frontend = 375
cargo test   --workspace                           # add DB-gated integration tests (needs DATABASE_URL + upstream stubs spun up inside the harness)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt    --check --all
cargo bench                                        # optional
```

### 12.3 Integration-test conventions

- Share `tests/common/mod.rs`: `database_url()` and `build_state(url)`.
- Every DB-bound test calls `database_url()` first and returns silently when unset, so CI without Postgres stays green.
- The only DB-free file is `auth_test.rs` (4 tests on the bearer middleware).
- Test full HTTP request/response cycles end-to-end where possible (`tower::ServiceExt::oneshot` against the router). Verify both status codes and JSON shape. Cover error paths (404, 409, 422).
- Concurrency tests use two tokio tasks racing for the same bed / same slot, asserting exactly one returns 200 and the other returns 409 — proves `SELECT … FOR UPDATE` actually prevents double-booking.

### 12.4 Unit-test conventions

- Place tests in a `#[cfg(test)] mod tests` block at the bottom of the source file.
- Descriptive names: `test_<function>_<scenario>` (e.g., `test_bed_status_occupied_to_available_fails`).
- Test both success and failure paths. State machines need at least one test per legal and illegal transition.
- Use model factory methods (`Patient::new`, `Identifier::mrn`, `RTTClockEvent::started`, …) over building structs by hand.
- For Tantivy: `tempfile::tempdir()` for the index path so the OS cleans up.

---

## 13. Build Sequence (Implementation Tasks)

Ordered, layered build sequence to recreate the **PAS Axum v0.43.0 / patient-administration-system-frontend v0.1.0** baseline from a clean checkout. Same-layer tasks parallelize; later layers depend on earlier ones. After every layer, run the gates in §12.2.

Layers 0–8 land the PAS Axum binary up through the HL7 v2 ADT^A07 demote-inpatient surface (v0.43.0). Layer 9 lands the Loco-rs sibling crate (`patient-administration-system-frontend` v0.1.0). Layer 10 is workspace-level release engineering.

### Layer 0 — Workspace scaffolding (blocking, single agent)

- **T0.1 Cargo workspace.** Create the workspace root `Cargo.toml` with three members. Resolver 2. Hoist `[profile.release]` (LTO, single codegen unit, strip) and `[profile.bench]` (inherits release) to the root. One shared `Cargo.lock`, one shared `target/`.
- **T0.2 PAS Axum `Cargo.toml`.** Populate exactly the dependency set in §10. Three bins (`patient-administration-system`, `pas-seed`, plus the `pas-migrate` bin from the migration crate), one example (`interchange`), three benches (`adt_bench`, `scheduling_bench`, `waitlist_bench`).
- **T0.3 Migration crate skeleton.** `migrations/Cargo.toml` + `migrations/src/lib.rs` (re-exports `MigratorTrait`) + `migrations/src/main.rs` (`pas-migrate` CLI: `up` / `down` / `fresh` / `status`). Reads `DATABASE_URL` via `dotenvy`.
- **T0.4 PAS Axum source skeleton.** `src/lib.rs` declares every module from §6.6 and re-exports `Error` / `Result`. `src/main.rs` is a minimal Axum server (stubs allowed). `src/error.rs` defines the 11-variant `Error` enum with `thiserror` plus helper constructors (`Error::not_found`, `Error::validation`, `Error::invalid_transition`).
- **T0.5 AGENTS skeleton.** Create `AGENTS.md` + `AGENTS/{index,architecture,models,restful,matching,testing,interchange}.md` + `AGENTS/share/{overview,technology,auditability,availability,observability,privacy,restful,match-search-merge}.md`. Headers + stubs now; bodies fill in alongside their layers.

### Layer 1 — Domain models (parallel, after Layer 0)

Pure-Rust types in `src/models/<name>.rs` with `serde` derives, factory methods, inline `#[cfg(test)] mod tests`. **No DB / IO dependencies.** Grounded in [`AGENTS/models.md`](AGENTS/models.md).

- **T1.1 Shared value types — `models/mod.rs`.** `Gender`, `NameUse`, `AddressUse`, `Address`, `ContactPointSystem`, `ContactPointUse`, `ContactPoint`, `Iso4217`, `Money`, `TimeRange`.
- **T1.2 Identity — `patient.rs`, `identifier.rs`.** `HumanName`, `EmergencyContact`, `Patient` (incl. `mpi_id`, `additional_names`, `deceased`, `marital_status`, `Patient::new`, `full_name`). `Identifier`, `IdentifierUse`, `IdentifierType` (UPPERCASE serde). Typed factories per supported national scheme: `Identifier::mrn` (local), `nhs` (UK), `nir` (France), `tsi` (España), `ihi` (Ireland), `hcn` (Northern Ireland), `ssn` (US). System-URI constants exported alongside each factory — see §4.0.1 + §7.2.
- **T1.3 Workforce — `practitioner.rs`.** `Practitioner`, `PractitionerRole`, `Department`.
- **T1.4 Resources — `facility.rs`.** `Facility`, `Ward`, `Room`, `Bed`, `BedStatus` + `try_transition_to`. **Test the `Occupied → Available` rejection.**
- **T1.5 ADT — `encounter.rs`, `admission.rs`.** `Encounter`, `EncounterClass`, `EncounterStatus` + `try_transition_to`. `Admission`, `Transfer`, `Discharge`, `BedAssignment` (with `release(&mut)` + `is_active()`).
- **T1.6 Scheduling — `schedule.rs`, `appointment.rs`.** `Schedule`, `ScheduleOwner`, `Slot`, `SlotStatus`. `Appointment`, `AppointmentStatus` + `try_transition_to`, `CancellationReason`.
- **T1.7 Waitlist + RTT — `waitlist.rs`, `rtt.rs`.** `Referral`, `WaitlistEntry`, `Priority` (`Ord` + snake_case), `WaitlistStatus`. `RTTPathway` (with `DEFAULT_BREACH_WEEKS = 18`, `is_breaching`), `RTTStatus`, `RTTClockEvent` + factories. Free function `compute_active_weeks`.
- **T1.8 Communication, Billing, Consent.** `LetterTemplate`, `GeneratedLetter`, `DeliveryChannel`, `LetterStatus`. `Account`, `Charge`, `Invoice` (`finalize` only valid from `Draft`), `InvoiceStatus`, `Payment`, `PaymentMethod`. `Consent`, `ConsentType`, `ConsentStatus`, `is_active(today)`.

### Layer 2 — Persistence (after Layer 1)

- **T2.1 Initial migration plus follow-ons.** `migrations/src/m20260520_000001_init.rs` creates the original 29 tables with the indexes from §9.2. Inline `JSONB` for repeated child collections on `patients` / `practitioners`. Four follow-on migrations bring the schema to the 32 tables in §9.1: `m20260525_000002_outbox_dlq` (`outbox_dead_letters`, v0.5), `m20260526_000003_appointment_series` (`appointment_series`, v0.9), `m20260527_000004_coverage` (`coverages`, v0.10), `m20260528_000005_patient_replaced_by` (nullable `patients.replaced_by`, v0.11).
- **T2.2 SeaORM entities.** `src/db/entities/*.rs` — one file per table (32 + `mod.rs`).
- **T2.3 Repositories.** One file per aggregate cluster. CRUD honors soft-delete (default filter `deleted_at IS NULL`; explicit `include_deleted` to surface tombstones).
- **T2.4 Audit + outbox repositories.** Accept any `&C: ConnectionTrait` so callers compose with primary writes in the same transaction.
- **T2.5 DB module.** `connect(url)` (pool 2–10, 8 s connect timeout, 60 s idle, `sqlx_logging` off), `with_txn<F, R>(db, f) -> Result<R>`.

### Layer 3 — Service modules (parallel, after Layer 2)

- **T3.1 ADT — `src/adt/`.** `admit`, `transfer`, `discharge`. Each in one DB transaction with `SELECT … FOR UPDATE` on the bed row. Transfers close the old assignment + flip the old bed to `Cleaning`; discharges close the active assignment + flip the bed to `Cleaning`. **No silent bed substitution.**
- **T3.2 Scheduling — `src/scheduling/`.** `book_slot`, `cancel`, `check_in`, `mark_no_show`, `complete`. Booking: `FOR UPDATE` on the slot, assert `Free`, per-patient overlap check (half-open), transition `Busy`, insert `Appointment(Booked)`.
- **T3.3 Waitlist + RTT — `src/waitlist/`.** Add/remove/list-breaches. RTT `start_clock`, `pause_clock(reason)`, `resume_clock`, `stop_clock(reason)`. Validators enforce event-log invariants.
- **T3.4 Resources — `src/resources/`.** `ward_occupancy(ward_id)`, `set_bed_status`.
- **T3.5 Billing — `src/billing/`.** `open_account` (one `Open` per patient invariant, currency fixed), `post_charge` (use `PostChargeInput` struct — keep clippy `too_many_arguments` happy), `finalize_invoice` (sum charges; currency match), `post_payment` (partial OK; status flips `Finalized → PartiallyPaid → Paid`).
- **T3.6 Communication — `src/communication/`.** `generate_letter` with **strict** required-variable check. `mark_sent` / `mark_failed`.

### Layer 4 — Cross-cutting (parallel, after Layer 3)

- **T4.1 Validation — `src/validation/`.** E.164 phone, address rules, charge amount (positive, currency match), appointment time (`start < end`), RTT clock-event ordering. **National healthcare-identifier validators (v0.15)**: `validate_nhs_number` (Mod 11), `validate_nir` (Mod 97 with Corsica `2A`/`2B` substitution), `validate_tsi` (alphanumeric envelope), `validate_ihi` (7 digits), `validate_hcn` (10 digits), plus the `validate_identifier(&Identifier)` dispatch helper that `validate_patient` calls over every `Patient.identifiers` entry.
- **T4.2 Privacy — `src/privacy/`.** `mask_value`, `mask_patient`, `export_patient`, `require_consent` helper.
- **T4.3 Search — `src/search/`.** Tantivy schema: `id`, `family_name`, `given_names`, `birth_date`, `mrn`, `postal_code`. `SearchEngine::new(path)`, `index_patient`, `delete_patient`, `search`. Index at `SEARCH_INDEX_PATH`. Inline tests use `tempfile::tempdir()`.
- **T4.4 Streaming — `src/streaming/`.** `EventPublisher` trait, `InMemoryEventPublisher`, `FluvioEventPublisher` stub, `DomainEvent` enum (every state change). `dispatcher::run(db, publisher, interval)` + `dispatcher::tick`.
- **T4.5 Observability — `src/observability/`.** `init(&Config)`: `tracing-subscriber` with `EnvFilter` + fmt layer, always. When `cfg.otlp_endpoint` is `Some(url)`, additionally build an OTLP `SpanExporter` (HTTP/protobuf via reqwest), wrap in a batch processor on the tokio runtime, register a `tracing_opentelemetry::layer()` alongside fmt. Service name from `cfg.otel_service_name` (default `pas-axum`). Setup failures are non-fatal — fall back to fmt-only with a `warn!`.
- **T4.6 Config — `src/config/`.** `Config::from_env()` reads every variable in §11.1.

### Layer 5 — REST + FHIR API (after Layer 3)

- **T5.1 REST router + state — `src/api/rest/`.** `state.rs::AppState` (services + DB + publisher + optional Tantivy via `with_search`). `mod.rs` exports `ApiResponse<T>`, `ApiError`, `router(state)`. `auth.rs` bearer-token middleware (`/api/health` always exempt; mismatch → 401 + envelope).
- **T5.2 REST handlers — `handlers.rs` + `routes.rs`.** All ~85 routes from README's endpoint table. Patient CRUD + search + masked + export + audit; encounter CRUD + cancel + status; ADT; scheduling (incl. bulk slots); waitlist + RTT + breaches; resources; workforce; billing; communication; consent; admin/outbox/unpublished; health. Error → status mapping per §6.7.
- **T5.3 FHIR R5 — `src/api/fhir/`.** `resources.rs` (`FhirPatient` / `FhirEncounter` / `FhirAppointment` / `FhirPractitioner` / `FhirSchedule` / `FhirSlot` / `FhirLocation` with bidirectional converters). `operation_outcome.rs`. `handlers.rs`: POST/GET/PUT/DELETE for `Patient`; POST/GET for `Encounter` + `Appointment`; GET for the four read-only types; `GET /fhir/Patient?_count=N`.
- **T5.4 FHIR Bundle writes — `POST /fhir`.** Accept `type: batch` or `type: transaction`. **Transaction is genuinely all-or-nothing** (see §4.8.2). Defer Tantivy updates to after commit.
- **T5.5 Wire `main.rs`.** Load config, init observability, connect DB, build state, build the merged router (`router.merge(fhir_router).merge(dashboard_routes).merge(SwaggerUi::...)`). Spawn the outbox dispatcher (`tokio::spawn` + `dispatcher::run` every 2 s). Add `TraceLayer`, `CorsLayer` (`CORS_ORIGINS` allowlist; permissive + warn when unset), `CompressionLayer`. Apply bearer middleware inside trace/cors/compress so 401 still gets those layers.

### Layer 6 — Tests, benches, infra

- **T6.1 Integration tests — `tests/`.** Share `tests/common/mod.rs` (`database_url`, `build_state`). Write the 19 files listed in [`AGENTS/testing.md`](AGENTS/testing.md): `health`, `auth`, `patient_crud`, `practitioner`, `adt_flow`, `scheduling_flow`, `waitlist_rtt_flow`, `billing_flow`, `letter_flow`, `consent_flow`, `fhir_write`, `concurrency` (two-way races: exactly one 200, one 409), `outbox_dispatcher`.
- **T6.2 Benchmarks — `benches/`.** Three Criterion suites: `adt_bench` (`BedStatus::try_transition_to` full cycle), `scheduling_bench` (`TimeRange::overlaps` at 16…1024 ranges), `waitlist_bench` (`compute_active_weeks` at 4…256 events). `harness = false` on each.
- **T6.3 Docker.** `Dockerfile` multi-stage (builder + slim runtime, non-root), `Dockerfile.test`, `docker-compose.yml` (Postgres + PAS), `docker-compose.test.yml` (Postgres only), `.dockerignore`, `.env.example`.
- **T6.4 Demo + seed.** `src/bin/seed.rs` (1 facility, 1 ward, 1 room, 3 `Available` beds, 2 practitioners, 1 patient, 1 letter template). `demo.sh` walks admit → ward-occupancy → transfer → discharge → audit history → letter generation against a running server.
- **T6.5 Docs fill-in.** Flesh out every `AGENTS/*.md` from the stubs created in T0.5. README with project overview, quick-start, full endpoint table, configuration table, curl examples per headline flow.

### Layer 7 — v0.2 Interchange + FHIR expansion

- **T7.1 Interchange module — `src/interchange/`.** `PatientRow` (flat projection, fixed field order — TSV/CSV depends on it). `From<&Patient>` + `into_partial_patient`. Submodules `json`, `xml` (`quick-xml`), `tsv` (`csv` with `b'\t'`), `csv` (RFC 4180). Document lossy fields in `AGENTS/interchange.md`.
- **T7.2 REST interchange endpoints.** `GET /api/patients/export.{json,xml,tsv,csv}` (cap 10k active). `POST /api/patients/import` sniffs `Content-Type`. **Idempotent**: skip by `id`. Response `{ inserted, skipped, failed }`.
- **T7.3 FHIR R5 read expansion.** `GET /fhir/Practitioner/{id}`, `/fhir/Schedule/{id}`, `/fhir/Slot/{id}`, `/fhir/Location/{id}`. `GET /fhir/Patient?_count=N` collection Bundle.
- **T7.4 Examples.** `examples/{patient-single,patients,fhir-patient,fhir-bundle,fhir-transaction-bundle}.json`, `examples/patients.{xml,tsv,csv}`, `examples/hl7-adt-{a01,a02,a03,a28}.txt`, `examples/interchange.rs` (`cargo run --example interchange` round-trips three patients through all four flat formats).
- **T7.5 Integration test.** `tests/interchange_test.rs` — POST import in all 4 formats → idempotent skip on re-import → exports round-trip → imported patients Tantivy-searchable.

### Layer 8 — v0.3 Interop + Dashboard + Workspace

- **T8.1 HL7 v2 module — `src/hl7v2/`.** `parser`, `encoder`, `mapping` (`pid_from_patient` / `patient_from_pid` with escapes honored), `ack` (AA / AE / AR), `escape`. Standard delimiters only; segments `MSH`, `EVN`, `PID`, `PV1`.
- **T8.2 HL7 v2 HTTP endpoints.** `POST /api/hl7/v2/{parse, patient, admit, transfer, discharge}`. All respond with a v2 ACK envelope. Exact-MRN dedup on A01 / A28; MSA-3 reports `matched existing patient <uuid>` when dedup fires.
- **T8.3 MLLP TCP listener.** `src/hl7v2/mllp.rs` (framing: `\x0B` start, `\x1C` end, `\x0D` CR). `src/hl7v2/listener.rs` (`MllpServer::serve`) — opt-in via `HL7V2_MLLP_BIND`. Dispatches via `axum::Router::oneshot` against the existing handlers — **zero handler refactoring**. **Bearer auth does not apply on the MLLP path.**
- **T8.4 Outbound MLLP — `src/streaming/hl7v2_publisher.rs`.** `Hl7v2MllpPublisher` implements `EventPublisher`: decodes payload, hydrates Patient + Bed, builds `ADT^A01/A02/A03` via `encode_adt_*`, opens TCP, writes the MLLP frame, reads ACK. `Ok` only on `MSA|AA`. Non-ADT events drop silently. Opt-in via `HL7V2_OUTBOUND_PEER`.
- **T8.5 FHIR transaction rollback.** Refactor `POST /fhir` into `process_batch_bundle` + `process_transaction_bundle`. Transaction path opens a single `DatabaseTransaction`, threads `&txn` through generic `create_*_in_db<C: ConnectionTrait>` helpers, commits only when every entry succeeds. Defer search-index updates until after commit. Integration test `tests/fhir_bundle_test.rs` proves: rollback removes prior good entry; diagnostic names the offending index; happy-path transaction commits both entries with `201 Created`.
- **T8.6 OpenAPI completion — `src/api/openapi.rs`.** `#[utoipa::path]` on all 86 `pub async fn` handlers. `ToSchema` derives on every request/response struct (shim domain enums with `#[schema(value_type = String)]`). 19 tags. Mount `SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi())`.
- **T8.7 Ops dashboard — `src/api/dashboard.rs` + `templates/`.** Tera templates baked via `include_str!`. `GET /dashboard` (full page) + four fragment endpoints. HTMX live-refresh (`hx-get` + `hx-trigger="load, every 10s"`). **First load works without JavaScript.** Reuse existing services + repos.
- **T8.8 Lily Design System markup.** Refactor every dashboard template to use Lily semantic class names. Tighten ARIA. Lily is **headless** — zero CSS — so the `<style>` block in `layouts/main.html` does the styling.
- **T8.9 Quality + dep audit.** `cargo fmt --all`. `cargo clippy --workspace --all-targets -- -D warnings` clean. Remove unused direct deps (15 of them; see §10) and unused dev-deps (`mockall`, `tokio-test`). Narrow `tower` features to `"util"`, `axum` to `["macros"]`.

### Layer 9 — `patient-administration-system-frontend` (Loco-rs sibling crate)

A separate workspace member at `../patient-administration-system-frontend/`. **Reads via sea-orm; writes only by proxying to the PAS Axum API.** Boots with `cargo run -- start` on port 5150.

- **T9.1 Loco-rs scaffold.** `patient-administration-system-frontend/Cargo.toml` with `loco-rs = "0.14.1"`, `sea-orm`, `tera`, `axum-extra = { version = "0.10", features = ["cookie"] }`, `reqwest`, `serial_test` (dev-dep). Crate-level `#![allow(clippy::result_large_err)]` (Loco's error enum is wide). `migration` dep by path.
- **T9.2 `Hooks` trait — `src/app.rs`.** Implement every method: `boot`, `routes`, `register_tasks`, `connect_workers`, `truncate`, `seed`. Truncate + seed are **no-ops** — patient-administration-system-frontend never mutates the shared schema.
- **T9.3 Read-side models — `src/models/`.** sea-orm projections: `patient`, `ward`, `bed`, `audit`, `outbox`, `rtt`, `schedule`, `letter_template`, `occupancy`. Read-only.
- **T9.4 Controllers + Tera views.** `controllers/{dashboard, patients, wards, rtt, admissions, appointments, letters, health}.rs`. Templates under `assets/views/`. Every template uses Lily markup.
- **T9.5 PAS API client — `src/services/pas_api.rs`.** Thin `reqwest` client (10 s timeout). Helpers: `admit`, `book_slot`, `generate_letter`. Shared `post_envelope` unwraps the PAS `ApiResponse` envelope and surfaces non-2xx with the parsed `error.message`. Base URL via `PAS_API_URL`.
- **T9.6 CSRF middleware — `src/csrf.rs`.** Double-submit-cookie pattern. `ensure_token(jar) -> (String, CookieJar)` reads or mints a `pas_csrf` cookie (`SameSite=Strict; HttpOnly; Path=/`, value = `Uuid::new_v4().simple()`). `verify_token(&jar, &form.csrf_token)` does a length-checked byte XOR (constant-time apart from length; token length is constant). Mismatch / missing cookie / empty field → `loco_rs::Error::BadRequest` → HTTP 400. `PAS_COOKIE_SECURE=1` adds the `Secure` flag.
- **T9.7 Tests.** 9 unit tests in `csrf::tests`. 13 integration tests in `tests/dashboard_smoke.rs` using Loco's `boot_test::<App>()` (gated on `DATABASE_URL`, `#[serial]` so each Loco boot binds a port serially): dashboard chrome + Lily markup, HTMX fragment isolation, patient list, ward detail, RTT cockpit, admit/book/letter forms (CSRF cookie set on GET; POST without valid token → 400), unknown-ward 404, `/_health` JSON shape.

### Layer 10 — Release engineering

- **T10.1 Workspace CHANGELOG.** `../CHANGELOG.md` at the workspace root coordinates the state of every member crate at each point in time; entries link out to per-crate `CHANGELOG.md` for detail.
- **T10.2 Repo unification (one-time, optional).** If the PAS Axum crate started life as a standalone repo, merge its history into the workspace root via `git subtree add --prefix=patient-administration-system-rust-crate/ <source> <commit> --squash`. A single `git log` at the workspace root then surfaces every commit forward.
- **T10.3 Doc audit pass.** On every release, sweep `AGENTS.md`, every `AGENTS/*.md` and `AGENTS/share/*.md`, the PAS README, and this `spec.md` for drift. Test counts, version pins, deferred-list items, and the technology stack table are the usual culprits.

### Layer 11 — Option C identity delegation (v0.44.0) — **SHIPPED 2026-06-01**

The big architectural shift: drop the local `Patient` + `Practitioner` tables and proxy every identity/workforce surface to `person-service` + `worker-service` over HTTP. Each sub-layer ends with workspace gates green; only Layer 11C is destructive.

**Status:** Shipped. The task list below is preserved verbatim as a historical implementation record; the work it describes corresponds to Phases 1 / 2 / 3a / 3b1–3b3 / 3c1–3c4 / 3d / 3e in `CHANGELOG.md`. Measured outcome (2026-06-01): 358 PAS lib tests pass; 11 inbound HL7 v2 handlers + 18 outbound trigger encoders shipped; route count 115 (REST) + 19 (FHIR R5); frontend v0.2.0 paired-release moves patient reads to `services/person_api.rs`.

> **DECISION CALLOUTS** (defaults chosen in Phase 0 doc rewrite; override before Layer 11A):
>
> 1. **Hard runtime dep, no cache.** PAS refuses to write a `person_id` / `worker_id` it cannot resolve upstream. Future LRU cache is a Phase 7 escape hatch.
> 2. **Forward-only migration; no backfill tool.** Existing local `patients` / `practitioners` rows are *lost* on the v0.44.0 upgrade unless an out-of-band export-to-upstream is performed first. The PAS does not ship one.
> 3. **Audit proxy.** `GET /api/patients/{id}/audit` merges upstream demographic-audit rows + local PAS-owned audit rows time-sorted.
> 4. **FHIR transaction Bundles dropping Patient/Practitioner.** `POST /fhir` with `type: transaction` **rejects** bundles that contain `Patient` or `Practitioner` entries with `400 + invalid: Patient/Practitioner entries are not supported in PAS transaction bundles; call person-service /fhir/Person and worker-service /fhir/Worker directly.` Batch Bundles still accept them by proxying each entry individually.
> 5. **MPI / person-service relationship.** This spec treats `person-service` + `worker-service` as the deferred MPI vision now realized. References to the abstract sister "MPI crate" elsewhere in the docs are deprecated and rewritten to point at these two services.

- **T11A.1 Upstream client trait + HTTP impl.** `src/upstream/mod.rs` defines `PersonClient` and `WorkerClient` async traits. `person_client.rs::HttpPersonClient` (and worker twin) wrap `reqwest::Client` with `UPSTREAM_TIMEOUT_SECS`, attach `Authorization: Bearer <MAIN_*_TOKEN>` when set, attach pass-through `X-User-Id` / `X-User-Ip` / `X-User-Agent` headers from a per-call `UserContext`. Errors map to `Error::Upstream` (transport / non-translatable non-2xx) or to `Error::NotFound` / `Error::Validation` / `Error::Conflict` when the upstream envelope is parseable.
- **T11A.2 Translation layer.** `src/upstream/translate.rs` — `Person ↔ Patient` and `Worker ↔ Practitioner` shape conversion. Drives every proxy handler in T11D.
- **T11A.3 Stub impls for tests.** `StubPersonClient` + `StubWorkerClient` — in-process HTTP servers on ephemeral `TcpListener` ports, same pattern as v0.14 `webhook_test.rs`. Used by every integration test that proxies upstream.
- **T11A.4 Unit tests.** Per-method tests for both clients against the in-process stubs: envelope unwrap, 4xx translation, transport error → `Upstream`, header attachment, signature attachment when configured.

- **T11B.1 Webhook inbound endpoints.** `POST /api/upstream/person-merged` and `POST /api/upstream/worker-merged`. HMAC-SHA256 signature check against `MAIN_*_WEBHOOK_SECRET` before any side-effect. Body `{source_id, target_id, at}`. Replay-safe on `X-Upstream-Event-Id` (stored in a small `upstream_inbox` table — adds one table; revisit count in §9.1).
- **T11B.2 Merge propagation logic.** Single DB transaction: `UPDATE encounters SET person_id = target WHERE person_id = source` (and the same for the 8 other person-bearing tables; 5 worker-bearing tables for the worker variant). One `audit_log` row per table touched, `action = "upstream_person_merged"` / `"upstream_worker_merged"`.

- **T11C.1 Destructive migration.** `migrations/src/m20260601_000006_delegate_identity.rs`:
    1. `DROP TABLE patients, practitioners, practitioner_roles, departments CASCADE` (CASCADE because the v0.11 `patients.replaced_by` partial index + every dependent table's FK go too).
    2. For each of 9 dependent tables, drop the FK constraint pointing at `patients.id`, then `ALTER COLUMN patient_id RENAME TO person_id`.
    3. For each of 5 dependent tables, drop the FK constraint pointing at `practitioners.id`, then `ALTER COLUMN practitioner_id RENAME TO worker_id`.
    4. Drop the local Tantivy index directory at `SEARCH_INDEX_PATH` from the filesystem (logged, not migrated).
    5. Create `upstream_inbox (event_id UUID PRIMARY KEY, received_at TIMESTAMPTZ NOT NULL)` for webhook replay-safety.
- **T11C.2 Entity teardown.** Delete `src/db/entities/{patient,practitioner,practitioner_role,department}.rs`; rename `patient_id → person_id` and `practitioner_id → worker_id` on every other entity. Delete `src/db/repositories/{patient,practitioner}.rs` and their references.
- **T11C.3 Model teardown.** Delete `src/models/{patient,practitioner,identifier}.rs`. Delete `src/validation/identifier.rs` and the national-identifier validators (`validate_nhs_number` etc.). Delete `src/search/` (Tantivy patient index).
- **T11C.4 Interchange teardown.** `src/interchange/` keeps the four format submodules but every `PatientRow` reader/writer now calls the upstream client per row instead of touching local DB.

- **T11D.1 Proxy REST handlers.** Every patient/practitioner REST handler in `src/api/rest/handlers.rs` is rewritten as a one-line proxy over `state.person_client` / `state.worker_client`. Wire shape preserved; translation via `upstream::translate`.
- **T11D.2 Proxy FHIR handlers.** Same for `src/api/fhir/handlers.rs` Patient + Practitioner handlers. `POST /fhir` transaction-Bundle path rejects Patient/Practitioner entries per Decision #4.
- **T11D.3 HL7 v2 PID/STF rewrites.** Every PID-bearing handler (17 ADT triggers + DFT^P03 + SIU + MFN^M02 PID inside the wire) replaces local `PatientRepository::dedup_or_create` with `state.person_client.match` + (on no match) `.create`. MFN^M02 STF segments handle worker identity the same way via `state.worker_client`. Audit + outbox events that fired locally for patient/practitioner CRUD **disappear**; encounter/admission/charge events keep firing.

- **T11E.1 Wire `main.rs`.** Build `HttpPersonClient` + `HttpWorkerClient` from env vars at startup. Ping each upstream `/api/health` once; abort startup with `Error::Config` if either returns non-2xx (production); log `warn!` and continue (test mode). Extend `/api/health` aggregator to include upstream status.

- **T11F.1 Test rewrites.** Delete `tests/patient_crud_test.rs` and `tests/patient_merge_test.rs`. Rewrite every other integration test to spin up `StubPersonClient` + `StubWorkerClient` HTTP stubs and seed them with a minimal person/worker before exercising the PAS flow. Add `tests/upstream_person_proxy_test.rs` (proxy round-trip), `tests/upstream_worker_proxy_test.rs`, `tests/upstream_merge_webhook_test.rs` (HMAC verify + merge propagation).
- **T11F.2 Bench rewrites.** Three Criterion suites stay (none touch Patient/Practitioner shape).

- **T11G.1 Final doc sweep.** Refresh §12.1 v0.44.0 counts with measured values. Bump Cargo.toml to `0.44.0`. Per-crate CHANGELOG entries. Workspace CHANGELOG entry pointing at the per-crate entries + this Layer 11 anchor.

---

## 14. Acceptance Criteria

### 14.1 Shipped baseline (v0.44.0, as of 2026-06-01) — currently passing

- `cargo build --workspace --all-targets` → zero errors, zero warnings.
- `cargo test --workspace --lib` → **358 PAS + 17 frontend = 375 passing**.
- `cargo test --workspace` with a live `DATABASE_URL` → above + DB-gated integration tests. Integration tests bind ephemeral ports for `StubPersonClient` / `StubWorkerClient` so no live upstream is required.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.
- `cargo bench` → runs the three Criterion suites end-to-end.
- A live `cargo run --bin patient-administration-system` requires `PERSON_SERVICE_URL` + `WORKER_SERVICE_URL` reachable; `/api/health` returns `{ status, database, ... }` and serves Swagger UI at `/swagger-ui`.
- `cargo run --bin patient-administration-system-frontend -- start` in the sibling crate serves `/dashboard` + `/patients`; patient pages render from `person-service` over HTTP and the three CSRF-protected write forms proxy back to the PAS Axum API and succeed when given valid tokens, return 400 when given invalid ones.

### 14.2 Historical baseline (v0.43.0) — kept for delta-tracking

- `cargo test --workspace --lib` → 497 PAS + 9 frontend = 506 passing. Patient/Practitioner/Identifier model + validator + Tantivy search tests accounted for the bulk of the v0.43 → v0.44 lib-test drop.
- `POST /api/patients` succeeds end-to-end: PAS proxies to `person-service POST /api/persons`, the returned `id` round-trips through `GET /api/patients/{id}` and `POST /api/admissions { person_id: <id>, bed_id: ... }`.
- `POST /api/upstream/person-merged` with a valid HMAC signature rewrites every PAS-owned row referencing the merge `source_id` to point at `target_id`, in one DB transaction, and ACKs 200. Replays of the same `X-Upstream-Event-Id` are no-ops.
- The PAS PostgreSQL schema contains no `patients`, `practitioners`, `practitioner_roles`, or `departments` tables. `\d patients` in psql returns "Did not find any relation named ...".

---

## 15. Document Map

| Document                                            | Purpose                                                            |
|-----------------------------------------------------|--------------------------------------------------------------------|
| `spec.md` (this file)                               | Comprehensive spec-driven-development document — single source of truth for scope, invariants, architecture, and build sequence. |
| [`README.md`](README.md)                            | Public surface: features, quick start, **full endpoint table**, configuration, status. |
| [`CHANGELOG.md`](CHANGELOG.md)                      | Per-release changelog (PAS Axum crate).                            |
| [`../CHANGELOG.md`](../CHANGELOG.md)                | Workspace-level changelog coordinating member-crate releases.      |
| [`AGENTS.md`](AGENTS.md)                            | Agent / human orientation for working on this crate.               |
| [`AGENTS/index.md`](AGENTS/index.md)                | Index of every AGENTS file.                                        |
| [`AGENTS/architecture.md`](AGENTS/architecture.md)  | Layered architecture, transactional outbox, four state machines, time, soft-delete, schema. |
| [`AGENTS/models.md`](AGENTS/models.md)              | Domain model reference, one section per `src/models/*.rs`.         |
| [`AGENTS/restful.md`](AGENTS/restful.md)            | REST envelope, error mapping, auth/CORS contract, headline shapes. |
| [`AGENTS/matching.md`](AGENTS/matching.md)          | Scheduling overlap, slot/bed concurrency, RTT clock arithmetic.    |
| [`AGENTS/interchange.md`](AGENTS/interchange.md)    | Bulk JSON/XML/TSV/CSV import-export, FHIR Bundles, HL7 v2 ADT.     |
| [`AGENTS/testing.md`](AGENTS/testing.md)            | Unit + integration test layout, benchmark suites, quality gates.   |
| [`AGENTS/share/*.md`](AGENTS/share)                 | Cross-cutting reference snippets (overview, technology, auditability, availability, observability, privacy, restful, match-search-merge). |
| [`AGENTS/national-person-identifiers.md`](AGENTS/national-person-identifiers.md) | Historical reference; the surface itself moved to `person-service` at v0.44.0. |
| `seed.md`                                           | Historical problem statement — do not modify.                      |
| **External:** `person-service` `spec.md`       | The upstream identity service that owns `Patient` shape, search, match, merge. |
| **External:** `worker-service` `spec.md`       | The upstream workforce service that owns `Practitioner` shape, search, match, merge. |

---

## 16. Maintenance

This spec is a **living document** — keep it aligned with the shipped surface. Update it when:

- Scope changes (new feature lands, or a deferred item gets explicitly killed).
- An invariant changes (rare; treat with the same care as a breaking schema change).
- The dependency stack changes (audit removes a crate, or a new one is introduced).
- The build sequence changes (a new layer, or a task's ordering shifts).
- Test counts / file counts change materially (refresh §12.1).

Past release history belongs in `CHANGELOG.md`, not here. This file describes **what the system is now** and **how to build it from scratch** — not how it got there.
