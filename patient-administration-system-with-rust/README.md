# Patient Administration System (PAS)

A production-grade Rust crate implementing a hospital Patient Administration System — the foundational, non-clinical system of record for healthcare workflow.

[![Rust](https://img.shields.io/badge/rust-1.99.1-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-five%20options-blue.svg)](#license)

## Overview

The PAS owns the administrative *who, when, where, what visit type, what bed, what bill* — it does not own the *what diagnosis*. It feeds clinical portals and downstream systems (EMR, billing, analytics).


## Features

- Patients are referred to by id. The PAS holds no people: identity lives in `person-service`, and the `/api/patients` routes pass through to it.
- ADT: admission, transfer, discharge with atomic bed allocation under `SELECT … FOR UPDATE`
- Scheduling: slot booking, cancellation, check-in, completion, no-show, slots generated from clinic templates. **Recurring appointment series** (v0.9): RFC 5545 subset (daily/weekly/monthly + INTERVAL + BYDAY + COUNT/UNTIL), atomic-overlap reject, cap 200 occurrences.
- Waitlist + RTT clock (NHS-style, configurable per-pathway breach threshold) with breach detection
- Resources: ward/bed occupancy snapshots, bed status transitions
- Billing: accounts, charges, invoices, payments (multi-currency via ISO 4217). **Coverage** (v0.10): insurance / self-pay / other payer records per patient, optionally linked to a billing account; FHIR R5 `Coverage` read at `/fhir/Coverage/:id`.
- Communication: Tera template letters with strict required-variable checking + delivery status. **SMS auto-send** (v0.8) via the `SmsProvider` trait — `LogSmsProvider` ships for dev/test; a real gateway is a one-trait-impl swap.
- Audit log (transactional, HIPAA-aligned) with per-entity and recent-activity query endpoints
- Domain events via transactional outbox pattern, polled by a background dispatcher
- RESTful API (Axum) with a response envelope, per-user tokens and optimistic concurrency (`ETag` and `If-Match`)
- FHIR R5 surface: `Patient`, `Practitioner`, `Schedule` and `Slot` read and write, with reads for `Appointment`, `Encounter`, `Location`, `Coverage` and others (see the table below). `POST /fhir` writes Bundles, including bookings and admissions
- PostgreSQL persistence via SeaORM, with a `pas-migrate` CLI
- Privacy: masked patient view, GDPR export, consent CRUD + revoke

## Quick Start

### Prerequisites

- Rust 1.99.1, the version pinned in `rust-toolchain.toml` (the 2024 edition and let-chains need at least 1.88)
- PostgreSQL 18+
- Podman (optional, recommended for local Postgres)

### Bootstrap a working server

```bash
git clone https://github.com/patient-administration-system/patient-administration-system.git
cd patient-administration-system/patient-administration-system-with-rust
cp .env.example .env

# 1. Start Postgres
podman compose up -d postgres

# 2. Apply migrations and load the demo ward (safe to run twice)
(cd migrations && cargo run --bin pas-migrate -- up)
cargo run --bin pas-seed

# 3. Run the API server (in another terminal)
cargo run --release

# 4. Walk the headline flows (admit, transfer, discharge)
./demo.sh
```

Server defaults to `http://localhost:8080`. `pas-seed` creates one facility (`DEMO`), one ward (`WARD-A`), one room and three beds, and nothing else: the PAS holds no people, so patients and staff are created in `person-service` and `worker-service`. `demo.sh` takes `PERSON_ID` (a real person's id from person-service); without it, it uses a made-up id, which is enough for the bed movements.

`pas-migrate` lives in the nested `migrations/` crate; subcommands are `up`, `down`, `fresh`, `status`.

### End-to-end demo

```bash
./demo.sh
```

Finds the seeded ward and beds via REST, then walks admit → ward-occupancy → transfer → discharge → audit history. Requires `jq`.

### Headline curl flow

```bash
# Health check
curl http://localhost:8080/api/health

# Find a bed in the seeded ward
FACILITY=$(curl -sS http://localhost:8080/api/facilities | jq -r '.data[] | select(.code == "DEMO") | .id')
WARD=$(curl -sS http://localhost:8080/api/facilities/$FACILITY/wards | jq -r '.data[] | select(.code == "WARD-A") | .id')
BED=$(curl -sS http://localhost:8080/api/wards/$WARD/beds | jq -r '.data[0].id')

# A patient is an id from person-service; a made-up one is enough for bed movements
PERSON=$(uuidgen | tr 'A-Z' 'a-z')

# Admit
curl -X POST http://localhost:8080/api/admissions \
  -H "Content-Type: application/json" \
  -H "X-User-Id: clinician-1" \
  -d "{\"person_id\":\"$PERSON\",\"bed_id\":\"$BED\"}"
```

## Technology Stack

A summary. `Cargo.toml` is the source of truth.

| Component            | Technology                                  | Purpose                                  |
| -------------------- | ------------------------------------------- | ---------------------------------------- |
| **Language**         | Rust 1.99.1 (pinned in `rust-toolchain.toml`), 2024 Edition                    | Systems programming, performance, safety |
| **Async Runtime**    | Tokio (`full`)                              | Asynchronous I/O and concurrency         |
| **Web Framework**    | Axum 0.8 (`macros`)                         | HTTP server and routing                  |
| **HTTP middleware**  | tower (`util`), tower-http (`cors`, `trace`, `compression-full`, `request-id`) | CORS / trace / compression / request id |
| **Templates**        | Tera 1.20                                   | Server-side rendering for letters + dashboard |
| **Database**         | PostgreSQL 18+                              | Persistence                              |
| **ORM**              | SeaORM 2.0 (`sqlx-postgres`, `runtime-tokio-rustls`, `macros`, `with-chrono`, `with-uuid`, `with-json`) | Data access |
| **Migrations**       | sea-orm-migration 2.0 + `pas-migrate` CLI   | Schema management                        |
| **Streaming**        | InMemoryEventPublisher (production + tests) + `Hl7v2MllpPublisher` (outbound ADT) + `WebhookEventPublisher` (v0.14, HMAC-signed HTTP POST) + `CompositePublisher` fan-out + `FluvioEventPublisher` stub | Event publishing |
| **API Docs**         | Utoipa 6.0 + `utoipa-swagger-ui` 10.0       | OpenAPI 3.1 annotations + Swagger UI     |
| **Serialization**    | Serde 1.0, serde\_json 1.0, csv 1.4         | JSON / CSV                               |
| **Logging / Tracing**| `tracing` 0.1, `tracing-subscriber` 0.3 (`env-filter`, `json`) | Structured logging (reads `RUST_LOG`) |
| **Observability**    | OpenTelemetry 0.33 + OTLP HTTP/protobuf via reqwest | Wires when `OTLP_ENDPOINT` is set |
| **Utilities**        | uuid 1 (`v4`, `v5`, `serde`), chrono 0.4 (`serde`), dotenvy 0.15, async-trait 0.1 | UUIDs, timestamps, env, async traits |
| **Error Handling**   | thiserror 2.0                               | Typed errors                             |
| **Money / Decimal**  | rust\_decimal 1.43 (`serde-with-str`)       | Exact monetary arithmetic                |
| **Testing**          | `assertables` 10, `tempfile` 3.27           | Rich asserts + temp directories          |
| **Benchmarking**     | Criterion 0.8 (`html_reports`, `async_tokio`) | Statistical performance benchmarks     |
| **Containerization** | Podman (multi-stage Containerfile), podman compose        | Deployment packaging                     |

## Project Structure

```
patient-administration-system-with-rust/
├── src/
│   ├── lib.rs                 Library root, public re-exports
│   ├── main.rs                Binary entry (Axum server)
│   ├── error.rs               Error enum, Result alias
│   ├── config/                Env/config loader
│   ├── models/                Pure-Rust domain types (no DB deps)
│   ├── db/                    SeaORM entities, repositories, audit + outbox
│   ├── adt/                   Admission/Transfer/Discharge service
│   ├── scheduling/            Slot booking, overlap detection
│   ├── waitlist/              Waitlist queue, RTT clock service
│   ├── resources/             Bed status, ward occupancy
│   ├── billing/               Charges, invoices, payments
│   ├── communication/         Tera letter rendering
│   ├── api/                   REST + FHIR R5 routers and handlers
│   ├── streaming/             Event publishers (in-memory, HL7 v2, webhook; Fluvio is a stub)
│   ├── validation/            Validators
│   ├── privacy/               Masking, GDPR export, consent helpers
│   └── observability/         tracing + OpenTelemetry
├── migrations/                SeaORM migration crate + `pas-migrate` CLI
├── tests/                     Integration tests (gated on DATABASE_URL)
├── benches/                   Criterion benchmarks (bed status, time overlap, RTT)
└── AGENTS/                    Architecture, models, API, scheduling, testing docs
```

## API Endpoint Reference

User context for audit logging is taken from headers: `X-User-Id`, `X-User-Ip`, `X-User-Agent`. When `API_TOKEN` is set, all `/api/*` endpoints (except `/api/health`) require `Authorization: Bearer <token>`. With `PAS_REQUIRE_AUTH=true` they need a per-user token; `../scripts/dev-auth.sh` sets that up for development.

The table lists the main routes. [`spec/openapi.json`](../spec/openapi.json) has all 298, and Swagger UI serves the same document at `/swagger-ui/`.

| Group | Method | Path |
|-------|--------|------|
| Health | GET | `/api/health` |
| Patient CRUD | POST · GET | `/api/patients` (`?limit=N`) |
| | GET · PUT · DELETE | `/api/patients/:id` (v0.40: DELETE refuses 409 if patient has open admission; writes outbox `PatientDeleted` that fans out as ADT^A23) |
| | GET | `/api/patients/search?q=…&limit=N` |
| | GET | `/api/patients/:id/masked` |
| | GET | `/api/patients/:id/export` |
| | GET | `/api/patients/:id/audit` |
| | GET | `/api/audit/recent` |
| | GET | `/api/audit/entity?entity_type=…&entity_id=…` |
| Ops | GET | `/api/admin/outbox/unpublished` |
| | GET | `/api/admin/outbox/dead-letters` (v0.5) |
| | POST | `/api/admin/outbox/dead-letters/:id/replay` (v0.5) |
| Encounter | GET | `/api/encounters/:id` |
| | GET | `/api/patients/:id/encounters` |
| | POST | `/api/encounters/:id/cancel` |
| | PUT | `/api/encounters/:id/status` |
| ADT | POST | `/api/admissions` |
| | POST | `/api/admissions/pre-admit` (v0.33; reserve bed + open Planned inpatient encounter; outbox fans out as ADT^A05) |
| | POST | `/api/admissions/change-to-inpatient` (v0.42; promote active ambulatory encounter to Inpatient + bed allocation; outbox fans out as ADT^A06) |
| | POST | `/api/admissions/cancel-pre-admit` (v0.35; release bed reservation + cancel Planned encounter; outbox fans out as ADT^A38) |
| | POST | `/api/admissions/:id/transfer` |
| | POST | `/api/admissions/:id/cancel-admit` (v0.38; cancel the open admission; outbox fans out as ADT^A11) |
| | POST | `/api/admissions/:id/cancel-transfer` (v0.31; reverses the most-recent transfer; outbox fans out as ADT^A12) |
| | POST | `/api/admissions/:id/leave-start` (v0.37; encounter InProgress → OnLeave; outbox fans out as ADT^A21) |
| | POST | `/api/admissions/:id/leave-end` (v0.37; encounter OnLeave → InProgress; outbox fans out as ADT^A22) |
| | POST | `/api/admissions/:id/discharge` |
| Scheduling | POST | `/api/appointment-series` (create a series and every occurrence in one transaction; an overlap refuses all of it) |
| | GET | `/api/appointment-series/:id` (v0.9 — series + linked occurrences) |
| | POST | `/api/appointment-series/:id/cancel` (v0.9 — cancel future occurrences) |
| | GET | `/api/patients/:id/appointment-series` (v0.9 — series per patient) |
| | GET | `/api/slots/:id` |
| | POST | `/api/slots/:id/book` |
| | GET | `/api/appointments?from=…&to=…` (at most 366 days) |
| | GET | `/api/appointments/:id` |
| | POST | `/api/appointments/:id/cancel` |
| | POST | `/api/appointments/:id/check-in` |
| | POST | `/api/appointments/:id/complete` |
| Waitlist + RTT | GET · POST | `/api/waitlist` (`?service=…`) |
| | GET · DELETE | `/api/waitlist/:id` |
| | GET | `/api/rtt/:id/breaching` |
| | POST | `/api/rtt/start` |
| | POST | `/api/rtt/:id/pause` · `/api/rtt/:id/resume` · `/api/rtt/:id/stop` |
| | GET | `/api/rtt/:id/weeks-waiting` |
| Resources | POST · GET | `/api/facilities` |
| | POST | `/api/wards` |
| | GET | `/api/wards/:id` · `/api/facilities/:id/wards` |
| | POST | `/api/rooms` |
| | POST | `/api/beds` (v0.27 — adds audit + outbox `BedCreated`) |
| | GET | `/api/beds/:id` · `/api/wards/:id/beds` · `/api/wards/:id/occupancy` |
| | PUT | `/api/beds/:id/status` (`If-Match`; `OutOfService` also emits outbox `BedRetired`) |
| Workforce | POST · GET | `/api/practitioners` |
| | GET · PUT · DELETE | `/api/practitioners/:id` |
| Billing | POST | `/api/accounts` |
| | POST | `/api/coverages` (v0.10) |
| | GET · PUT · DELETE | `/api/coverages/:id` (v0.10 — DELETE flips status to `cancelled`; never hard-deletes) |
| | GET | `/api/patients/:id/coverages` · `/api/accounts/:id/coverages` (v0.10) |
| | POST | `/api/charges` · `/api/invoices` · `/api/payments` |
| Communication | GET · POST | `/api/letter-templates` |
| | GET · PUT | `/api/letter-templates/:id` |
| | POST | `/api/letters/generate` |
| | GET | `/api/letters/:id` |
| | POST | `/api/letters/:id/sent` · `/api/letters/:id/failed` |
| Consent | POST · GET | `/api/patients/:id/consents` |
| | POST | `/api/consents/:id/revoke` |
| Interchange (v0.2) | GET | `/api/patients/export.json` · `/api/patients/export.xml` · `/api/patients/export.tsv` · `/api/patients/export.csv` |
| | POST | `/api/patients/import` (Content-Type picks JSON / XML / TSV / CSV) |
| HL7 v2 (v0.3) | POST | `/api/hl7/v2/parse` (echoes parsed message as JSON) |
| | POST | `/api/hl7/v2/batch` (v0.6 — accepts a batch envelope of up to 1000 ADT messages; returns per-message ACKs wrapped in one BHS/BTS envelope) |
| | POST | `/api/hl7/v2/patient` (ingest ADT^A28/A01; returns ACK^... text body) |
| | POST | `/api/hl7/v2/admit` (ingest ADT^A01; creates patient + admits to bed from PV1-3.3) |
| | POST | `/api/hl7/v2/pre-admit` (ingest ADT^A05; reserves bed from PV1-3.3 and opens a Planned inpatient encounter; no admission row; v0.32) |
| | POST | `/api/hl7/v2/change-to-inpatient` (ingest ADT^A06; promotes most-recent active ambulatory encounter to Inpatient + bed allocation; v0.41) |
| | POST | `/api/hl7/v2/change-to-outpatient` (ingest ADT^A07; demotes open inpatient admission to Outpatient; releases bed; encounter stays InProgress; v0.43) |
| | POST | `/api/hl7/v2/cancel-pre-admit` (ingest ADT^A38; releases the bed reservation and cancels the Planned inpatient encounter; v0.34) |
| | POST | `/api/hl7/v2/leave-start` (ingest ADT^A21; encounter InProgress → OnLeave; bed remains Occupied; v0.36) |
| | POST | `/api/hl7/v2/leave-end` (ingest ADT^A22; encounter OnLeave → InProgress; v0.36) |
| | POST | `/api/hl7/v2/delete-patient` (ingest ADT^A23; soft-deletes patient + drops from search; refuses if patient has any open admission; v0.39) |
| | POST | `/api/hl7/v2/register` (ingest ADT^A04; dedup-or-create patient, open Outpatient or Emergency encounter in Arrived status — no bed; v0.28) |
| | POST | `/api/hl7/v2/transfer` (ingest ADT^A02; finds open admission by MRN, transfers to bed from PV1-3.3) |
| | POST | `/api/hl7/v2/discharge` (ingest ADT^A03; finds open admission by MRN and discharges) |
| HL7 v2 ADT-lifecycle (v0.4) | POST | `/api/hl7/v2/update` (ingest ADT^A08; merges PID demographics over the existing patient row) |
| | POST | `/api/hl7/v2/cancel-admit` (ingest ADT^A11; releases bed, flips bed to Cleaning, cancels encounter) |
| | POST | `/api/hl7/v2/cancel-transfer` (ingest ADT^A12; reverses the most-recent transfer; restores patient to origin bed; v0.30) |
| | POST | `/api/hl7/v2/cancel-discharge` (ingest ADT^A13; reinstates the most-recent discharge to the original bed) |
| | POST | `/api/hl7/v2/merge` (ingest ADT^A40 — merge source patient identified by MRG-1 MRN into survivor identified by PID; v0.18) |
| | POST | `/api/hl7/v2/dft` (ingest DFT^P03 — post one or many charges from FT1 segments; v0.20 makes all-or-nothing across multi-FT1 messages; auto-creates an open billing account when none exists; v0.19) |
| | POST | `/api/hl7/v2/mfn-staff` (ingest MFN^M02 — master file notification for practitioner roster; MAD/MUP/MDL via MFE+STF pairs, atomic per message; v0.24) |
| | POST | `/api/hl7/v2/mfn-location` (ingest MFN^M05 — master file notification for bed roster; MAD/MUP/MDL via MFE+LOC pairs, LOC-1 PL-typed `<room>^^<bed>`, atomic per message; v0.26) |
| HL7 v2 SIU (v0.16 + v0.17) | POST | `/api/hl7/v2/schedule-book` (ingest SIU^S12 — book appointment from SCH + PID; AA ACK reports the assigned filler id) |
| | POST | `/api/hl7/v2/schedule-reschedule` (ingest SIU^S13 — change start/end of appointment identified by SCH-2 filler uuid; overlap-excluding check) |
| | POST | `/api/hl7/v2/schedule-modify` (ingest SIU^S14 — update reason of appointment identified by SCH-2 filler uuid) |
| | POST | `/api/hl7/v2/schedule-cancel` (ingest SIU^S15 — cancel appointment identified by SCH-2 filler uuid) |
| FHIR R5 | POST | `/fhir` (Bundle; no entry type can be written through it yet, see [below](#fhir-r5)) |
| | POST | `/fhir/Patient` · `/fhir/Practitioner` · `/fhir/Schedule` · `/fhir/Slot` |
| | GET · PUT · DELETE | `/fhir/Patient/:id` · `/fhir/Practitioner/:id` · `/fhir/Schedule/:id` · `/fhir/Slot/:id` (last three v0.21) |
| | GET | `/fhir/Encounter/:id` · `/fhir/Appointment/:id` · `/fhir/Location/:id` |
| | GET | `/fhir/Patient?_count=N` (collection `Bundle`) |
| | GET | `/fhir/Coverage/:id` (v0.10 — FHIR R5 Coverage read; write through a `POST /fhir` Bundle) |

## API Examples

The PAS holds no people, so anything that reads or writes a person goes through `person-service` and needs `PERSON_SERVICE_URL`. Without it, patient import and export, FHIR `Patient` and letters answer `503`, and HL7 ADT messages answer with an `AE` ACK. The examples for bed movements, scheduling, waiting lists, RTT and billing take a person's id and do not need the service.

### Health

```bash
curl http://localhost:8080/api/health
```

### ADT — Admit, Transfer, Discharge

```bash
# Admit a patient to a bed
curl -X POST http://localhost:8080/api/admissions \
  -H "Content-Type: application/json" \
  -d '{
    "person_id": "11111111-1111-1111-1111-111111111111",
    "bed_id":    "22222222-2222-2222-2222-222222222222"
  }'

# Transfer to a new bed. If-Match carries the ETag from GET /api/admissions/<admission-id>
curl -X POST http://localhost:8080/api/admissions/<admission-id>/transfer \
  -H "Content-Type: application/json" \
  -H 'If-Match: <etag>' \
  -d '{ "new_bed_id": "33333333-3333-3333-3333-333333333333" }'

# Discharge. The version moved with the transfer, so read the ETag again
curl -X POST http://localhost:8080/api/admissions/<admission-id>/discharge \
  -H "Content-Type: application/json" \
  -H 'If-Match: <etag>' \
  -d '{ "disposition": "home" }'
```

### Scheduling

```bash
# Book a slot
curl -X POST http://localhost:8080/api/slots/<slot-id>/book \
  -H "Content-Type: application/json" \
  -d '{ "person_id": "<uuid>" }'

# Cancel an appointment. If-Match carries the ETag from GET /api/appointments/<id>
curl -X POST http://localhost:8080/api/appointments/<id>/cancel \
  -H "Content-Type: application/json" \
  -H 'If-Match: <etag>' \
  -d '{ "reason": "patient_request" }'
```

### Waitlist + RTT

```bash
# Add to waitlist
curl -X POST http://localhost:8080/api/waitlist \
  -H "Content-Type: application/json" \
  -d '{
    "person_id":      "<uuid>",
    "target_service": "cardiology",
    "priority":       "routine"
  }'

# Start an RTT clock
curl -X POST http://localhost:8080/api/rtt/start \
  -H "Content-Type: application/json" \
  -d '{ "person_id": "<uuid>", "target_service": "cardiology" }'

# Pause an RTT pathway
curl -X POST http://localhost:8080/api/rtt/<pathway-id>/pause \
  -H "Content-Type: application/json" \
  -d '{ "reason": "patient_holiday" }'
```

### Resources

```bash
# Ward occupancy snapshot
curl http://localhost:8080/api/wards/<ward-id>/occupancy
```

### Billing

```bash
# Open an account
curl -X POST http://localhost:8080/api/accounts \
  -H "Content-Type: application/json" \
  -d '{ "person_id": "<uuid>", "currency": "USD" }'

# Post a charge
curl -X POST http://localhost:8080/api/charges \
  -H "Content-Type: application/json" \
  -d '{
    "account_id":      "<uuid>",
    "code":            "99213",
    "description":     "Office visit",
    "amount_value":    "120.00",
    "amount_currency": "USD"
  }'

# Finalize an invoice
curl -X POST http://localhost:8080/api/invoices \
  -H "Content-Type: application/json" \
  -d '{ "account_id": "<uuid>", "charge_ids": ["<uuid>", "<uuid>"] }'

# Post a payment
curl -X POST http://localhost:8080/api/payments \
  -H "Content-Type: application/json" \
  -d '{
    "invoice_id":      "<uuid>",
    "amount_value":    "120.00",
    "amount_currency": "USD",
    "method":          "card",
    "reference":       "TXN-9001"
  }'
```

### Communication

```bash
# Generate a letter from a template
curl -X POST http://localhost:8080/api/letters/generate \
  -H "Content-Type: application/json" \
  -d '{
    "template_id":    "<uuid>",
    "person_id":      "<uuid>",
    "appointment_id": "<uuid>",
    "channel":        "print"
  }'
```

### Interchange — JSON, XML, TSV

Bulk patient data in JSON, XML, TSV and CSV, passed through to `person-service`. See [`AGENTS/interchange.md`](AGENTS/interchange.md) for the formats and [`examples/`](examples/) for working payloads.

```bash
# Export every active patient (capped at 10k rows)
curl -s http://localhost:8080/api/patients/export.json
curl -s http://localhost:8080/api/patients/export.xml
curl -s http://localhost:8080/api/patients/export.tsv
curl -s http://localhost:8080/api/patients/export.csv

# Bulk import (idempotent — existing ids are skipped). Format is picked from
# Content-Type: application/json, application/xml, or text/tab-separated-values.
curl -X POST http://localhost:8080/api/patients/import \
  -H 'Content-Type: application/json' \
  --data-binary @examples/patients.json

curl -X POST http://localhost:8080/api/patients/import \
  -H 'Content-Type: application/xml' \
  --data-binary @examples/patients.xml

curl -X POST http://localhost:8080/api/patients/import \
  -H 'Content-Type: text/tab-separated-values' \
  --data-binary @examples/patients.tsv

curl -X POST http://localhost:8080/api/patients/import \
  -H 'Content-Type: text/csv' \
  --data-binary @examples/patients.csv

# Response shape:
# { "success": true, "data": { "inserted": 2, "skipped": 1, "failed": 0 } }
```

### FHIR R5

```bash
# Create / read / update / soft-delete a Patient
curl -X POST http://localhost:8080/fhir/Patient \
  -H 'Content-Type: application/json' \
  --data-binary @examples/fhir-patient.json
curl http://localhost:8080/fhir/Patient/{id}

# Collection Bundle of recent patients
curl 'http://localhost:8080/fhir/Patient?_count=20'

# Read other administrative resources
curl http://localhost:8080/fhir/Practitioner/{id}
curl http://localhost:8080/fhir/Schedule/{id}
curl http://localhost:8080/fhir/Slot/{id}
curl http://localhost:8080/fhir/Location/{id}   # PAS bed
```

`POST /fhir` writes a `transaction` or `batch` Bundle. An entry can create a `Schedule`, a `Slot` or a `Coverage`, book an `Appointment` (`status` `booked`, one `slot`, a `subject`), or admit an `Encounter` (`status` `in-progress`, `class` `IMP`, a `subject`, and the bed as one `location`). Bookings and admissions meet the rules of the REST routes: a slot must be free and a bed available. Each write is audited and publishes an event in the same transaction. A `transaction` is all or nothing, and an entry can name a `Schedule` or `Slot` in the same Bundle by the `urn:uuid:` `fullUrl` of its entry. A `batch` answers each entry on its own, with HTTP 200. `Patient` and `Practitioner` entries are refused with `400`, because `person-service` and `worker-service` own them and a write there cannot share this transaction: use `POST /fhir/Patient` or `POST /fhir/Practitioner`. Any other resource type, an `Encounter` that is not inpatient, a `PUT` or `DELETE` entry, and `ifNoneExist` answer `501`.

### HL7 v2 ADT

For interop with legacy clinical systems, v0.3 ships a pipe-delimited HL7 v2 ingest endpoint. The first cut supports `MSH`, `EVN`, `PID`, and `PV1` segments with the standard delimiter set `|^~\&`. See [`AGENTS/interchange.md`](AGENTS/interchange.md) for the segment-to-domain mapping and the [`examples/hl7-adt-a28.txt`](examples/hl7-adt-a28.txt) sample payload.

```bash
# Inspect a message's structure as JSON
curl -X POST http://localhost:8080/api/hl7/v2/parse \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a28.txt

# Ingest an ADT^A28 (add person info); the response body is a v2 ACK envelope
curl -X POST http://localhost:8080/api/hl7/v2/patient \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a28.txt
# →  MSH|^~\&|PAS|FAC|EMR|FAC|...|ACK|MSG-EMR-001|P|2.5\r
#    MSA|AA|MSG-EMR-001\r

# Ingest an ADT^A01 (admit). The PV1-3.3 bed code must match an existing bed.
# On success: creates the patient AND admits them through AdtService::admit,
# so the bed flips to "occupied" and an EncounterAdmitted event is published.
curl -X POST http://localhost:8080/api/hl7/v2/admit \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a01.txt

# ADT^A02 (transfer). Identifies the open admission by MRN (PID-3.1) and
# transfers to the destination bed in PV1-3.3.
curl -X POST http://localhost:8080/api/hl7/v2/transfer \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a02.txt

# ADT^A03 (discharge). Identifies the open admission by MRN and discharges.
curl -X POST http://localhost:8080/api/hl7/v2/discharge \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a03.txt
```

ACK codes: `AA` = accepted, `AE` = application error (well-formed but rejected — e.g. PID-5 missing), `AR` = rejected before processing (parse failure).

#### MLLP TCP listener

Real hospital networks ship HL7 v2 over MLLP, not HTTP. Set `HL7V2_MLLP_BIND=0.0.0.0:2575` (the conventional port) to enable a TCP listener that consumes MLLP-framed (`\x0b<message>\x1c\x0d`) ADT messages and routes them through the same handlers as the HTTP endpoints. The ACK is returned in the same MLLP framing.

```bash
HL7V2_MLLP_BIND=0.0.0.0:2575 cargo run --bin patient-administration-system

# Send a frame using Python:
python3 - <<'PY'
import socket
msg = b"MSH|^~\\&|EMR|FAC|PAS|FAC|20260523120000||ADT^A28|MSG1|P|2.5\r" \
      b"EVN|A28|20260523120000\r" \
      b"PID|1||MRN-MLLP-001^^^FAC^MR||Doe^Jane||19900115|F\r"
s = socket.create_connection(("localhost", 2575))
s.sendall(b"\x0b" + msg + b"\x1c\x0d")
print(s.recv(4096))
PY
```

## Configuration

Configuration is loaded from environment variables (or `.env`).

| Variable             | Description                                                       | Default          | Required |
| -------------------- | ----------------------------------------------------------------- | ---------------- | -------- |
| `DATABASE_URL`       | PostgreSQL connection string                                      | -                | Yes      |
| `SERVER_HOST`        | HTTP server bind address                                          | `0.0.0.0`        | No       |
| `SERVER_PORT`        | HTTP server port                                                  | `8080`           | No       |
| `PERSON_SERVICE_URL` | Base URL of `person-service`. Unset, anything that needs a person is refused (see [API Examples](#api-examples)) | - | No |
| `PAS_REQUIRE_UPSTREAMS` | `true` refuses to start without `PERSON_SERVICE_URL` and `WORKER_SERVICE_URL`. Set it in every real deployment | `false` | No |
| `RUST_LOG`           | Logging level (e.g., `info`, `debug`)                             | `info`           | No       |
| `OTLP_ENDPOINT`      | OpenTelemetry OTLP collector endpoint (HTTP/protobuf, e.g. `http://otel-collector:4318/v1/traces`). When set, every `tracing` span is exported to the collector. When unset (default), fmt layer only — no network egress. | - | No |
| `OTEL_SERVICE_NAME`  | `service.name` resource attribute on every exported span. Override to disambiguate replicas / environments in the collector. | `pas-axum` | No |
| `PAS_SMS_PROVIDER`   | SMS provider selector (v0.8). `none` = no auto-send, SMS letters stay `pending`. `log` = log each outbound message to `tracing` and flip the letter to `sent`. Real production gateways are added by the consumer via the `SmsProvider` trait. Unknown values fall back to `none`. | `none` | No |
| `PAS_RATE_LIMIT_RPM` | Per-IP rate-limit (v0.12) sustained refill rate. Set `0` to disable the middleware entirely. | `600` (= 10 req/sec) | No |
| `PAS_RATE_LIMIT_BURST` | Per-IP rate-limit bucket capacity. A client may fire `burst` requests back-to-back before being gated to the sustained rate. Ignored when `PAS_RATE_LIMIT_RPM=0`. | `60` | No |
| `API_TOKEN`          | If set, bearer-auth is enforced on every `/api/*` except `/api/health` | -          | No       |
| `CORS_ORIGINS`       | Comma-separated allowlist; empty/unset = permissive (with warn)   | -                | No       |
| `HL7V2_MLLP_BIND`    | `host:port` for the MLLP TCP listener (HL7 v2 ADT). Unset = off   | -                | No       |
| `HL7V2_OUTBOUND_PEER`| `host:port` of a downstream MLLP receiver for outbound ADT messages | -            | No       |
| `PAS_OUTBOX_MAX_RETRIES`| Per-event retry budget for the outbox dispatcher. After this many consecutive failed publishes, the row is moved to `outbox_dead_letters`. Set to `0` to disable dead-lettering. | `10` | No |
| `PAS_WEBHOOK_URL`    | v0.14 — outbox webhook destination URL. When set, `WebhookEventPublisher` POSTs every `DomainEvent` as JSON. Combines with `HL7V2_OUTBOUND_PEER` via `CompositePublisher` fan-out when both are set. | unset (disabled) | No |
| `PAS_WEBHOOK_SECRET` | v0.14 — HMAC-SHA256 secret for the webhook. When set, every POST carries `X-PAS-Signature: sha256=<hex>` over the raw body. | unset | No |
| `PAS_WEBHOOK_TIMEOUT_SECS` | v0.14 — webhook request timeout in seconds. Keep strictly shorter than the dispatcher 2 s poll for responsive retries. | `10` | No |

When the server is running, the **complete OpenAPI spec** (every REST + FHIR + HL7 v2 endpoint — patients, ADT, scheduling, billing, communication, audit, consent, interchange, FHIR Bundles, HL7 v2) is served at `/api-docs/openapi.json` with Swagger UI at `/swagger-ui`. An at-a-glance ops view (ward occupancy, RTT breaches, outbox unpublished count, recent audit) is served at `/dashboard` — Tera-rendered server-side on first load, then HTMX polls four fragment endpoints every 10s to keep each panel live. Works without JavaScript (initial render embeds every fragment inline); HTMX is a progressive enhancement. Markup uses [Lily Design System](https://github.com/LilyDesignSystem/lily) headless HTML class names (`header`, `footer`, `panel`, `data-table*`, `badge`, `alert`, `code`).

A parallel **Loco-rs front-end** lives at [`../patient-administration-system-frontend`](../patient-administration-system-frontend/) — separate crate, shares this app's PostgreSQL database, ports the same dashboard + adds patient list/detail pages. Builds clean against Loco 0.14.1 (Hooks impl, ViewEngine<TeraView> extractors, sea-orm read-side projections). Boot with `cargo run -- start` from the `patient-administration-system-frontend/` directory; browse to `http://localhost:5150/dashboard`.

## Development

```bash
# Build
cargo build

# Test (library tests only)
cargo test --lib

# Lint
cargo clippy

# Format
cargo fmt

# Format check (CI)
cargo fmt --check
```

## Migrations

Migrations live as a nested SeaORM crate at `migrations/` with a thin CLI binary `pas-migrate`:

```bash
# All subcommands honour DATABASE_URL from .env or the environment.
(cd migrations && cargo run --bin pas-migrate -- up)        # apply pending
(cd migrations && cargo run --bin pas-migrate -- status)    # show state
(cd migrations && cargo run --bin pas-migrate -- fresh)     # drop + re-apply
(cd migrations && cargo run --bin pas-migrate -- down)      # revert all
```

Migrations are one file each in `migrations/src/`; the count is in [`spec/14-implementation-status.md`](../spec/14-implementation-status.md). Repeated child collections are stored inline as `JSONB` columns rather than as separate tables.

## Testing

Live counts are in [`spec/14-implementation-status.md`](../spec/14-implementation-status.md).

**Unit tests** (`cargo test --lib`) cover domain models, repositories, services, HL7 v2 parsing and framing, FHIR converters, the OpenAPI document, validation, privacy and configuration.

**Integration tests** are the files in `tests/`, most named after the task that added them. Those that need PostgreSQL are `#[ignore]`d and need `DATABASE_URL`:

```bash
cargo test -- --ignored --test-threads=1
```

**Benchmarks:** three Criterion suites under `benches/`.

| File                          | What's measured                                                |
| ----------------------------- | -------------------------------------------------------------- |
| `benches/adt_bench.rs`        | `BedStatus::try_transition_to` full cycle                      |
| `benches/scheduling_bench.rs` | `TimeRange::overlaps` across linear scans (16…1024 ranges)     |
| `benches/waitlist_bench.rs`   | `compute_active_weeks` at varying event counts (4…256)         |

Run with `cargo bench` or e.g. `cargo bench -- adt`.

**Quality gates:**

```bash
cargo test --lib                                          # unit tests
cargo test                                                # add the integration tests (most need DATABASE_URL and `--ignored`)
cargo clippy --workspace --all-targets -- -D warnings     # zero warnings
cargo fmt --check --all                                   # formatting clean
```

## Status

**Version:** v0.49.0 — library + `patient-administration-system` binary + `pas-seed` binary + `pas-migrate` binary. See [`CHANGELOG.md`](CHANGELOG.md) for the per-release summary.

What is built and what is not changes with every release, so it is generated rather than written here: see [`spec/14-implementation-status.md`](../spec/14-implementation-status.md), and [`tasks.md`](../tasks.md) for the open work.

## License

Available under any one of MIT, Apache-2.0, BSD-3-Clause, GPL-2.0-only or GPL-3.0-only, at your choice. See [`LICENSE.md`](../LICENSE.md).
