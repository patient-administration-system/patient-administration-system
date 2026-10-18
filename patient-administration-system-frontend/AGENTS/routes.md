# Routes

Every route this crate serves. The full table also lives in [`README.md`](../README.md#routes); this doc adds template paths and (for writes) the PAS Axum endpoint each one proxies to.

> **v0.2 target (Option C).** The route table itself is unchanged. What changes is the *data source* behind some routes: `/patients`, `/patients/{id}`, anything showing a patient name or practitioner name on `/wards/{id}` / `/rtt` / dashboard, and the patient/practitioner pickers on the three write forms move from sea-orm reads to `services/main_person_api` + `services/main_worker_api` reqwest reads against the upstream services. The CSRF write surface and the PAS Axum endpoints the writes proxy to are unchanged in shape (the PAS Axum back end internally re-proxies identity fields to the upstream services).

## Read routes (GET)

| Path | Controller | Template | Notes |
|---|---|---|---|
| `/_health` | `controllers/health.rs` | (JSON, no template) | `{ service, version, database }` |
| `/dashboard` | `controllers/dashboard.rs` | `assets/views/dashboard/index.html` | Full server-side render with all four panels inlined (first paint works without JavaScript) |
| `/dashboard/wards` | `controllers/dashboard.rs` | `assets/views/dashboard/_wards.html` | HTMX fragment — ward occupancy |
| `/dashboard/outbox` | `controllers/dashboard.rs` | `assets/views/dashboard/_outbox.html` | HTMX fragment — unpublished outbox count |
| `/dashboard/audit` | `controllers/dashboard.rs` | `assets/views/dashboard/_audit.html` | HTMX fragment — last 10 audit rows |
| `/patients` | `controllers/patients.rs` | `assets/views/patients/index.html` | Patient list |
| `/patients/{id}` | `controllers/patients.rs` | `assets/views/patients/show.html` | Patient detail; 404 on unknown id with a Lily-styled empty state |
| `/wards/{id}` | `controllers/wards.rs` | `assets/views/wards/show.html` | Ward detail (beds + occupants); 404 on unknown id |
| `/rtt` | `controllers/rtt.rs` | `assets/views/rtt/index.html` | Active 18-week pathways, sorted worst-breach-first |
| `/admissions/new` (GET) | `controllers/admissions.rs` | `assets/views/admissions/new.html` | Sets `pas_csrf` cookie + hidden field |
| `/appointments/new` (GET) | `controllers/appointments.rs` | `assets/views/appointments/new.html` | Sets `pas_csrf` cookie + hidden field; 14-day flat slot picker |
| `/letters/new` (GET) | `controllers/letters.rs` | `assets/views/letters/new.html` | Sets `pas_csrf` cookie + hidden field |

## Write routes (POST)

Every POST is CSRF-protected via `src/csrf.rs::verify_token`. Mismatch / missing cookie / empty form field → HTTP **400** *before* any PAS API call. See [csrf.md](csrf.md).

| Path | Controller | PAS Axum REST endpoint proxied | Audit + outbox |
|---|---|---|---|
| `/admissions/new` (POST) | `controllers/admissions.rs::create` | `POST /api/admissions` | `EncounterAdmitted` |
| `/appointments/new` (POST) | `controllers/appointments.rs::create` | `POST /api/slots/{id}/book` | `AppointmentBooked` |
| `/letters/new` (POST) | `controllers/letters.rs::create` | `POST /api/letters/generate` | `LetterGenerated` (+ optional SMS auto-send audit entries when the channel is `Sms` and a provider is configured on the PAS Axum side) |

## HTMX progressive enhancement

The three dashboard fragment endpoints use `hx-get` + `hx-trigger="load, every 10s"` on the corresponding panel. They also render inline on `GET /dashboard` so the first paint does not depend on JavaScript.

A small live-indicator dot appears during in-flight fragment requests. It's CSS-driven; no JS hooks beyond HTMX itself.

## What this crate does *not* serve

- JSON API endpoints (other than `/_health`) — those live in the PAS Axum binary.
- FHIR R5 endpoints — same.
- HL7 v2 endpoints (HTTP or MLLP) — same.
- OpenAPI / Swagger UI — same.

The PAS Axum binary at `:8080` is the right surface for any machine-readable interaction.

## See Also

- [architecture.md](architecture.md) — handler shape, write-flow proxy contract
- [csrf.md](csrf.md) — what `verify_token` checks
- [../README.md](../README.md#routes) — same table for non-AGENTS readers
- [../spec.md](../spec.md#71-full-route-table) — same table inside the spec
