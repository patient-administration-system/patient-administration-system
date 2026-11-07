# Changelog

All notable changes to `patient-administration-system-frontend` are documented in this file. Format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this
crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Calls to the PAS API and `person-service` carry the `X-Request-Id` of the request they serve (AXA-6), so a failing page can be followed across services.
- `GET /_readiness` answers `503` naming `database` while the database cannot be reached (AXA-4). It is axa's route, as is `/_ping`; `/_health` still answers `200`, and says `unreachable`. Point an orchestrator at `/_readiness`.
- The `pas_csrf` cookie carries `Secure` by default everywhere production-like (AXA-5). `PAS_COOKIE_SECURE` overrides it either way, and a value that is neither yes nor no leaves the default.
- Moved from SeaORM to sqlx (SQLX-1): axa's `sqlx` feature, literal SQL in `src/models/`, and `db migrate` and `db status` directly on the pool. Each query asks for the same rows as before, and pages sort as before.
- Read PAS API refusals as RFC 9457 problems with `axa::problem` (FR-206).
- Replaced Loco with [axa](https://crates.io/crates/axa) (FR-205). Every page renders byte for byte as before; the footer now says Axum.
- Moved to axa 0.5. Every response now has a `content-security-policy`, the policy in `src/app.rs`, which allows the pages' inline styles and HTMX from unpkg. `--help` and `--version` describe this service, and `check` prints its settings with no database.
- A release build with no `APP_ENV` is production, a production `DATABASE_URL` must set `sslmode` unless it is loopback or `DB_ALLOW_INSECURE=true`, and a client that does not finish its headers in 30 seconds is dropped (`HEADER_READ_TIMEOUT_SECONDS`). The server speaks HTTP/1 only, holds at most 10,000 connections (`MAX_CONNECTIONS`), and `db reset`, `db down` and `db truncate` are refused by `cli::parse` before the service runs.
- Logs show a route template, or `unmatched`, and not the path. A `404` or `400` no longer logs as an error.
- Configuration comes from the environment and `src/app.rs`. `config/` is gone, and `APP_ENV` replaces `LOCO_ENV`, which axa still reads.
- `db reset`, `db down` and `db truncate` refuse. The service does not migrate on boot.
- A missing file under `/static` is a `404`. It was a `200` with the fallback page.
- `Access-Control-Allow-Origin: *` is gone, as is `X-Powered-By`. Every response now has `X-Content-Type-Options`, `X-Frame-Options` and `Referrer-Policy`, and production has HSTS and a 60-second request timeout.
- A template change needs a restart: the hot reload came with Loco.
- Development binds `localhost`, not `0.0.0.0`. Set `BINDING` to change it.

### Fixed

- Production boots. `config/production.yaml` quoted its port, which Loco 1.2 refused, and required a `404.html` that was never in the repository.

## [Unreleased]

### Changed
- `loco-rs` 1.2 and `sea-orm` 2.0 (DEP-2), with Loco's default features off (no `auth`, no `db-sqlite`). Template data goes through `tpl::json!`, which writes plain numbers because Tera 2 cannot compare `serde_json`'s `arbitrary_precision` numbers.

### Fixed
- The application could not start: `mailer.smtp` lacked `host`, `queue.kind: Null` is not valid in Loco 0.14, Loco's default routes duplicated `/_health`, and no Tera view engine was installed, so every page returned 500. `/_ping` is now served by this app.
- All 13 smoke tests now pass with `DATABASE_URL` set.

## [0.2.0] — 2026-05-31

Paired with PAS Axum **v0.44.0** Option C identity delegation. The
`patients` table no longer exists in the shared schema, so the
frontend's patient projection moves to an HTTP client against
`person-service` (`:5180`).

### Added

- `src/services/person_api.rs` — reqwest client with
  `list_persons(limit)` and `get_person(id)` against
  `PERSON_SERVICE_URL` (defaults to `http://localhost:5180`).
  Tolerates both PAS-envelope (`{ success, data }`) and bare-array
  upstream response shapes. 3 inline unit tests.

### Changed

- `src/models/patient.rs` — entire module rewritten. The local
  `patients` sea-orm projection is gone; `list_active_patients` and
  `find_patient_by_id` now delegate to `services::person_api` and
  translate the upstream `Person` JSON into the existing
  `PatientCard` / `PatientDetail` view-models so the templates stay
  unchanged. `list_active_patients` returns an empty `Vec` on
  upstream failure (best-effort dashboard render). 5 inline unit
  tests cover the name / identifier / telecom / address extractors
  including FHIR `line[]` arrays and legacy `line1` strings.
- `controllers/patients.rs`, `controllers/admissions.rs`,
  `controllers/appointments.rs`, `controllers/letters.rs` — call
  sites lose the `&ctx.db` argument since the new model functions
  source their data over HTTP.

### Removed

- Sea-orm entity for `patients` (the table no longer exists after
  PAS v0.44 destructive migration).

## [0.1.0] — 2026-05-24

First release. A Loco-rs front-end for the Patient Administration
System, served separately from the PAS Axum API on port 5150.

### Added — page surface

- **Ops dashboard** at `GET /dashboard` — full server-side render with
  four Lily-marked-up panels (ward occupancy, RTT breaches, outbox
  unpublished count, last 10 audit rows). HTMX polls each panel
  fragment every 10s with a small live-indicator dot during in-flight
  requests; first paint works without JavaScript.
- **Patient list + detail** at `GET /patients` and `GET /patients/{id}`.
- **Ward detail** at `GET /wards/{id}` — beds with current occupants,
  links to patient pages.
- **RTT cockpit** at `GET /rtt` — active 18-week pathways sorted
  worst-breach-first with weeks-waiting + breach badges.
- **Admission form** at `GET/POST /admissions/new` — patient + bed
  pickers; submit calls the PAS Axum API.
- **Appointment booking** at `GET/POST /appointments/new` — single
  flat slot picker (14-day horizon) + patient picker.
- **Letter composer** at `GET/POST /letters/new` — template + patient
  + channel pickers; renders the generated letter inline.
- **Health probe** at `GET /_health` — `{ service, version, database }`.

### Added — write-flow architecture

Every POST handler proxies through the PAS Axum REST API
(`POST /api/admissions`, `POST /api/slots/{id}/book`,
`POST /api/letters/generate`) so audit rows + outbox events land in
the system of record. patient-administration-system-frontend never writes directly to the shared
DB; it only reads via sea-orm.

`services/pas_api.rs` carries a thin reqwest client (10s timeout) with
a `PasApiError` enum (HTTP / parsed non-2xx status / malformed body).
Base URL via `PAS_API_URL` env var, default `http://localhost:8080`.

### Added — security

- **CSRF middleware** (`src/csrf.rs`) — double-submit-cookie pattern on
  every POST form. `pas_csrf` cookie carries a 128-bit UUID with
  `SameSite=Strict; HttpOnly; Path=/`, mirrored as a hidden
  `csrf_token` form field. Verification uses a constant-time byte
  comparison; mismatch returns HTTP 400 before any side effects.
- **`PAS_COOKIE_SECURE` env var** — set to `1` / `true` / `yes` in
  production so the cookie also carries the `Secure` flag. Off by
  default so local-dev over plaintext HTTP still works.

### Added — design system

Every template uses Lily Design System *headless* HTML class names
(`header`, `footer`, `panel`, `data-table*`, `badge`, `alert`, `code`).
Lily ships zero CSS — all styling lives in
`assets/views/layouts/main.html`'s `<style>` block. Swap that block for
Tailwind / Sass / anything else without touching the templates.

### Added — tests

- 9 unit tests on the `csrf` module (constant-time compare,
  ensure/verify paths, `Secure`-flag env-var toggle).
- 13 integration smoke tests in `tests/dashboard_smoke.rs` covering:
  dashboard chrome + Lily markup, HTMX fragment isolation, patient
  list, RTT cockpit, admission/appointment/letter forms (incl.
  CSRF: GET sets cookie + hidden field, POST without valid csrf
  returns 400), unknown-ward 404, `/_health` JSON shape.
- All gated on `DATABASE_URL` so `cargo test` works offline.

### Added — operational

- Cargo workspace integration: `patient-administration-system-frontend` is a sibling of
  `patient-administration-system-rust-crate` under one root, sharing
  the migration crate by path and a single `target/` build cache.
- Dep audit clean — no unused direct or dev dependencies.

[0.1.0]: https://github.com/joelparkerhenderson/patient-administration-system/releases/tag/patient-administration-system-frontend-v0.1.0
