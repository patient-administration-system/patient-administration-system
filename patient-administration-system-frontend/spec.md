# patient-administration-system-frontend — Specification

Comprehensive specification-driven-development document for the **Loco-rs front-end** of the Patient Administration System (PAS). This file is the **single source of truth** for what the front-end is, how it must behave, how it is architected, and the sequence in which it is built from a clean checkout to the current shipped baseline (`patient-administration-system-frontend` v0.1.0, paired with PAS Axum v0.43.0).

> **Target: v0.2.0 (paired with PAS Axum v0.44.0) — Option C identity delegation.**
> The PAS Axum back end ditches its local `patients` + `practitioners` tables and proxies identity to `person-service` + `worker-service`. This front-end therefore **loses its direct sea-orm read path** to patient + practitioner data — those reads move to two new reqwest clients (`src/services/main_person_api.rs` + `src/services/main_worker_api.rs`) that hit the upstream services directly with the user's auth context.
> The CSRF write surface is unchanged in shape (`/admissions/new`, `/appointments/new`, `/letters/new`) — those forms still proxy the user's POST through the PAS Axum API at `:8080`, which in turn proxies identity-bearing fields to the upstream services. The PAS Axum API remains the single write-fan-in. See PAS Axum `spec.md` §13 Layer 11 for the back-end migration and the workspace `CHANGELOG.md` Phase 0 entry.

Past release detail lives in [`CHANGELOG.md`](CHANGELOG.md) (this crate) and [`../CHANGELOG.md`](../CHANGELOG.md) (workspace). System-of-record contracts that this crate consumes live in [`../patient-administration-system-with-rust/spec.md`](../patient-administration-system-with-rust/spec.md).

---

## 1. Identity & Purpose

`patient-administration-system-frontend` is a **read-mostly human-facing web app** that sits alongside the PAS Axum API server. The two are deployed as **separate binaries on separate ports** and **share one PostgreSQL database**:

| Role | Crate | Port | Owns |
|------|-------|------|------|
| System of record (REST + FHIR + HL7 v2) | `patient-administration-system` (Axum) | `8080` | All writes, audit, outbox dispatcher |
| Human-facing UI | `patient-administration-system-frontend` (Loco-rs) | `5150` | Dashboard, patient pages, three CSRF-protected write forms |

**Invariant**: the front-end never writes directly to the shared schema. Every state-changing flow (admit / book slot / generate letter) proxies through the PAS Axum REST API so the audit + outbox + Tantivy invariants of the system of record cannot be bypassed.

The two binaries are workspace siblings (see workspace `Cargo.toml`) and share the migration crate `patient-administration-system-migrations` by path. Apply migrations once via `pas-migrate up`; both apps see the resulting schema.

---

## 2. Scope

### 2.1 In scope (v0.1.0)

- **Ops dashboard** at `GET /dashboard` — full server-side Tera render with four Lily-marked-up panels: ward occupancy, RTT breaches, outbox unpublished count, last 10 audit rows.
- **HTMX fragment endpoints** at `/dashboard/{wards,outbox,audit}` — each fragment refreshes the matching dashboard panel every 10 s via `hx-get`; first paint works without JavaScript.
- **Patient list + detail** at `GET /patients` and `GET /patients/{id}`.
- **Ward detail** at `GET /wards/{id}` — beds with current occupants, links to patient pages.
- **RTT cockpit** at `GET /rtt` — active 18-week pathways sorted worst-breach-first with weeks-waiting + breach badges.
- **Admission form** at `GET/POST /admissions/new` — patient + bed pickers; submit proxies `POST /api/admissions` on the PAS Axum API.
- **Appointment booking** at `GET/POST /appointments/new` — flat 14-day slot picker + patient picker; submit proxies `POST /api/slots/{id}/book`.
- **Letter composer** at `GET/POST /letters/new` — template + patient + channel pickers; submit proxies `POST /api/letters/generate`.
- **Health probe** at `GET /_health` — JSON `{ service, version, database }`.
- **CSRF protection** on every POST form (double-submit-cookie pattern).
- **Read-only sea-orm projections** in `src/models/` (patient, ward, bed, audit, outbox, rtt, schedule, letter_template, occupancy).
- **Loco-rs `Hooks` trait** fully implemented in `src/app.rs` (boot, routes, register_tasks, connect_workers, truncate, seed). The DB-mutating hooks (truncate, seed) are intentionally no-ops.

### 2.2 Out of scope (intentionally deferred)

Listed so a developer knows **not** to build them here:

- **Authentication / sessions / login UI.** Not in v0.1.0. Add via `axum-login` or Loco's auth helpers when needed. CSRF is in place so the foundation for authenticated write flows is ready.
- **Static asset pipeline (Tailwind, Sass).** `assets/static/` is empty by design; styling lives in a single `<style>` block in `assets/views/layouts/main.html`.
- **Direct DB writes.** All write paths go through the PAS Axum REST API. The Loco `truncate` and `seed` hooks are no-ops for this reason.
- **Background workers.** `register_tasks` and `connect_workers` are stubbed empty. The outbox dispatcher lives in the PAS Axum binary, not here.
- **Clinical content.** Same scope discipline as the PAS Axum binary: administrative views only — no diagnoses, no prescriptions, no vitals.
- **Replacing the PAS Axum binary.** The two are complementary. The front-end never owns the system of record.

### 2.3 Boundary with the PAS Axum binary

| Concern | Owner | Mechanism |
|---|---|---|
| Read paths (lists, details, dashboard panels) | `patient-administration-system-frontend` | sea-orm reads against the shared Postgres schema |
| Write paths (admit, book, generate letter) | `patient-administration-system` (Axum) | HTTP REST: `services/pas_api.rs` reqwest client proxies the form into the PAS API |
| Audit + outbox + Tantivy index | `patient-administration-system` (Axum) | Side-effects of every PAS REST write |
| Schema migrations | `patient-administration-system-migrations` | `pas-migrate up` from the with-rust project; both apps see the result |
| CSRF on the three write forms | `patient-administration-system-frontend` | Double-submit-cookie middleware in `src/csrf.rs` |

---

## 3. Stakeholders & Use Cases

| Stakeholder | Primary use cases |
|---|---|
| Hospital admin staff | Watch live ops dashboard for occupancy, breaches, outbox health |
| Front-desk clerks | Book an appointment through the booking form (CSRF-protected) |
| Ward nurses | Look up a ward, see beds + occupants, drill into a patient page |
| Operations / SRE | Refresh dashboard panels every 10 s via HTMX; spot stuck outbox rows |
| Compliance | Inspect the last 10 audit rows from the dashboard |
| AI agents / scripts | The PAS Axum REST API is the right surface; this UI is for humans |

---

## 4. Functional Requirements

### 4.1 Page surface

The full route table is the source of truth in [`README.md`](README.md#routes); this section describes contracts.

- **Every page handler** uses the Loco `ViewEngine<TeraView>` extractor and renders an `assets/views/<area>/<name>.html` template via `format::render().view(&v, "...", data)`.
- **Every template** uses [Lily Design System](https://github.com/LilyDesignSystem/lily) headless HTML class names. See §4.4.
- **Read paths** load sea-orm projections from `src/models/` — never the PAS Axum binary's full domain models. The projection layer is the boundary that lets the front-end iterate independently.
- **Write paths** load the same read projections for picker UIs (patient list, bed list, slot list, template list), then submit via `src/services/pas_api.rs` to the PAS Axum REST API.

### 4.2 Dashboard

`GET /dashboard` renders the full page server-side with four Lily-marked-up panels:

1. Ward occupancy
2. RTT breaches
3. Outbox unpublished count
4. Last 10 audit rows

Three of those four panels also have HTMX fragment endpoints (`/dashboard/wards`, `/dashboard/outbox`, `/dashboard/audit`) that re-render just the panel. Each fragment uses `hx-get` + `hx-trigger="load, every 10s"`. A small live-indicator dot appears during in-flight requests. **First paint works without JavaScript** — every fragment is also rendered inline in the full page; HTMX is a progressive enhancement.

### 4.3 Write forms — CSRF contract

The three POST routes are protected by the double-submit-cookie pattern in `src/csrf.rs`:

1. **On every form GET**, the handler calls `ensure_token(jar)` which reads (or mints) the `pas_csrf` cookie. Cookie attributes: `SameSite=Strict; HttpOnly; Path=/`. Value: 128-bit UUID formatted as 32 hex chars (`Uuid::new_v4().simple()`). The same token is embedded as a hidden `csrf_token` input in the form.
2. **On POST**, the handler calls `verify_token(&jar, &form.csrf_token)`. Bytes compared via a constant-time XOR (length-checked first; the token length is a constant so leaking it is acceptable). Mismatch / missing cookie / empty form field → `loco_rs::Error::BadRequest` → HTTP **400** *before* any side effects.
3. **In production**, set `PAS_COOKIE_SECURE=1` (or `true` / `yes`) so the cookie also carries the `Secure` flag. Off by default so local-dev over plaintext HTTP still works.

`SameSite=Strict` + `HttpOnly` mean a third-party site can neither read the cookie nor cause the browser to send it on a cross-origin POST, so an attacker has no way to supply a matching token.

### 4.4 Design system — Lily

Every template uses Lily Design System *headless* HTML class names:

- `header` / `footer` — page chrome
- `panel` (with `role="region"`) — dashboard cards
- `data-table` / `data-table-head` / `data-table-body` / `data-table-row` / `data-table-th` / `data-table-td` — tables
- `badge` (with `data-status="ok|warn|error"`) — status indicators
- `alert` (with `role="status"`) — inline diagnostics + empty states
- `code` — inline code

Lily ships **zero CSS**. All styling lives in `assets/views/layouts/main.html`'s `<style>` block — swap that block for Tailwind, a Sass build, or any other styling without touching the templates.

### 4.5 Health probe

`GET /_health` returns JSON `{ service, version, database }` where `service` = crate name, `version` = `CARGO_PKG_VERSION`, `database` = `"ok"` if a trivial `SELECT 1` succeeds else `"unreachable"`. Always served regardless of session / CSRF state.

---

## 5. Non-Functional Requirements / Invariants

These are **non-negotiable**. Every reviewer (human or agent) checks them.

| # | Invariant | Why |
|---|---|---|
| 1 | The front-end **never writes directly** to the shared PAS PostgreSQL database. Every write flow proxies through the PAS Axum REST API. | Audit + outbox invariants of the system of record cannot be bypassed. |
| 1b *(v0.2 target)* | The front-end **never reads patient or practitioner data directly from any database**. Those reads go through `services/main_person_api.rs` + `services/main_worker_api.rs` HTTP clients, with the user's pass-through auth headers attached. | Identity and workforce live in `person-service` / `worker-service` (PAS Axum v0.44.0 Option C); there is no local table to read. |
| 2 | The Loco `truncate` + `seed` hooks are **no-ops**. | Same as #1 — accidentally destructive hooks on a shared schema would corrupt the system of record. |
| 3 | Every POST form is CSRF-protected by the double-submit-cookie pattern. | Browser-borne CSRF is the realistic threat for a server-rendered UI. |
| 4 | First paint of every page works **without JavaScript**. HTMX is a progressive enhancement on top. | Resilience + accessibility + degradation under flaky networks. |
| 5 | Every template uses Lily Design System headless class names. Zero inline styles in templates. | Style swap (Tailwind / Sass / etc.) requires only one file change. |
| 6 | The migration crate is shared by path (`migration = { path = "../patient-administration-system-with-rust/migrations", … }`); this crate **does not declare its own migrations**. | Schema drift between the two binaries is impossible. |
| 7 | Stateless server: every request reads from the DB (or PAS API). Horizontal scaling by adding replicas. | Availability. |
| 8 | Crate-level `#![allow(clippy::result_large_err)]` is intentional. `loco_rs::Error` is a wide enum that trips the lint on every handler signature. | Keep clippy clean without splattering `allow` on every handler. |
| 9 | This crate **does not add a direct `axum` / `tower` / `tower-http` / `tracing` dependency**. Loco brings pinned versions of those; adding a second copy produces conflicting `FromRequest` impls. | Dependency graph hygiene. |

---

## 6. Architecture

### 6.1 Workspace placement

This crate is one of three workspace members rooted at `..`; v0.2 adds two upstream HTTP service dependencies (same as the PAS Axum back end):

| Crate / Service | Role | Port |
|---|---|---|
| `patient-administration-system` | Workflow service (REST + FHIR R5 + HL7 v2 over HTTP + MLLP) | 8080 |
| `patient-administration-system-migrations` | sea-orm migration crate; CLI is `pas-migrate` | n/a |
| `patient-administration-system-frontend` (this crate) | Loco-rs read-mostly UI | 5150 |
| `person-service` *(HTTP dep, external; v0.2 target)* | Person reads for patient pages + dashboard projections | 5180 |
| `worker-service` *(HTTP dep, external; v0.2 target)* | Worker reads for practitioner pickers + audit attributions | 5181 |

Workspace-level profile blocks (`[profile.release]`, `[profile.bench]`) live in the root `Cargo.toml`; per-member profile blocks are silently ignored by Cargo. One shared `target/`, one shared `Cargo.lock`.

### 6.2 Layout

```
patient-administration-system-frontend/
├── Cargo.toml              loco-rs + axum-extra + sea-orm + reqwest + …
├── CHANGELOG.md            per-release changelog (currently v0.1.0)
├── README.md               public surface — route table, env vars, CSRF design
├── spec.md                 this file
├── config/
│   ├── development.yaml    bind 5150, dev Postgres
│   ├── production.yaml     env-driven config for prod
│   └── test.yaml
├── src/
│   ├── main.rs             loco_rs::cli::main entry
│   ├── lib.rs              module declarations + crate-level lints
│   ├── app.rs              `Hooks` impl — app_name, app_version, boot, routes, register_tasks, connect_workers, truncate, seed
│   ├── csrf.rs             double-submit-cookie middleware
│   ├── controllers/
│   │   ├── dashboard.rs    full page + 3 HTMX fragment endpoints
│   │   ├── patients.rs     list + detail
│   │   ├── wards.rs        ward detail (beds + occupants)
│   │   ├── rtt.rs          RTT cockpit (active pathways, breaches)
│   │   ├── admissions.rs   GET form + POST submit (calls PAS API)
│   │   ├── appointments.rs GET form + POST submit
│   │   ├── letters.rs      GET compose form + POST generate
│   │   ├── health.rs       /_health
│   │   └── mod.rs
│   ├── models/             read-only sea-orm projections (one per area)
│   ├── services/pas_api.rs reqwest client for PAS REST writes
│   ├── views/              Rust-side template helpers (currently empty)
│   └── initializers/       boot-time wiring (currently empty)
├── assets/
│   ├── views/              Tera templates (layouts + per-area)
│   └── static/             served at /static/...
└── tests/
    └── dashboard_smoke.rs  13 integration tests (gated on DATABASE_URL)
```

### 6.3 Layered design

Strict three-layer service: `controller → model (read) | services/pas_api (write) → db (read) / PAS Axum REST (write)`.

- **controllers** (`src/controllers/`) — one file per area. Each controller exports a `routes() -> Routes` builder via `Routes::new().prefix("/x").add("/y", get(handler))`. Handlers use the Loco `ViewEngine<TeraView>` + `State<AppContext>` extractors and return `Result<Response>` via `format::render().view(&v, "template.html", json!({…}))`.
- **read models** (`src/models/`) — sea-orm projection queries. Read-only.
- **write services** (`src/services/pas_api.rs`) — thin reqwest client (10 s timeout) with a `PasApiError` enum (HTTP transport / parsed non-2xx status / malformed envelope body). Base URL via `PAS_API_URL`, default `http://localhost:8080`. Wraps the PAS `ApiResponse` envelope so the caller gets the `data` directly or a typed error.
- **CSRF middleware** (`src/csrf.rs`) — `ensure_token(jar) -> (String, CookieJar)` + `verify_token(&jar, &form_token) -> Result<()>`. Constant-time compare via XOR.

### 6.4 Loco-side patterns the scaffold uses

- **`Hooks` trait** fully implemented in `src/app.rs` (incl. `boot`, `routes`, `register_tasks`, `connect_workers`, `truncate`, `seed`). The DB-mutating hooks (`truncate`, `seed`) are intentionally no-ops — this crate never mutates the shared PAS schema.
- **`ViewEngine<TeraView>` extractor** on every page handler: `pub async fn index(ViewEngine(v): ViewEngine<TeraView>, State(ctx): State<AppContext>) -> Result<Response>`.
- **`format::render().view(&v, "template.html", data)`** with `data` as `serde_json::json!({…})`. Tera reads the template from `assets/views/`.
- **`Routes::new().prefix("/x").add("/y", get(handler))`** for the routing DSL; routes are aggregated in `Hooks::routes`.
- **Crate-level `#![allow(clippy::result_large_err)]`** in `src/lib.rs` and `src/main.rs` because `loco_rs::Error` is a wide enum that trips the lint on every handler signature.

---

## 7. API / Routing Specification

### 7.1 Full route table

| Method | Path | Description |
|---|---|---|
| GET | `/_health` | Liveness probe (`{ service, version, database }`) |
| GET | `/dashboard` | Ops dashboard — full page render |
| GET | `/dashboard/wards` | HTMX fragment: ward occupancy |
| GET | `/dashboard/outbox` | HTMX fragment: unpublished outbox count |
| GET | `/dashboard/audit` | HTMX fragment: last 10 audit rows |
| GET | `/patients` | Patient list |
| GET | `/patients/{id}` | Patient detail |
| GET | `/wards/{id}` | Ward detail (beds, occupants) |
| GET | `/rtt` | RTT cockpit — active pathways with weeks-waiting + breach flags |
| GET | `/admissions/new` | Admission form (patient + bed pickers; CSRF cookie set) |
| POST | `/admissions/new` | Submit admission (CSRF-verified; calls `POST /api/admissions`) |
| GET | `/appointments/new` | Booking form (slot + patient pickers; CSRF cookie set) |
| POST | `/appointments/new` | Submit booking (CSRF-verified; calls `POST /api/slots/{id}/book`) |
| GET | `/letters/new` | Letter compose form (template + patient + channel pickers; CSRF cookie set) |
| POST | `/letters/new` | Generate letter (CSRF-verified; calls `POST /api/letters/generate`) |

All write flows (POST endpoints) go through the PAS Axum API so audit + outbox events land in the system of record.

### 7.2 Error contract

- **CSRF mismatch / missing cookie / empty form field** → HTTP **400** (`loco_rs::Error::BadRequest`) before any side effects.
- **Unknown id on a detail page** (`/patients/{id}` / `/wards/{id}`) → HTTP **404** with a Lily-styled empty-state page.
- **PAS Axum API returns non-2xx** → the controller surfaces the parsed `error.message` from the PAS `ApiResponse` envelope in the form's error region; status remains 200 (the form re-renders with the diagnostic). Transport / parse failures surface as HTTP 500.

### 7.3 No JSON / FHIR / HL7 surface

The front-end exposes only Tera-rendered HTML (and the JSON `/_health` probe). All machine-readable surfaces (REST, FHIR R5, HL7 v2) live in the PAS Axum binary.

---

## 8. Data Model Specification

This crate **defines no schema** and adds no migrations. The migration crate (`patient-administration-system-migrations`) is the single source of truth and is shared by path; apply migrations once via `pas-migrate up` from the with-rust project.

`src/models/` holds **read-only sea-orm projections** sized to the templates that consume them:

| File | Projection of | v0.2 target (Option C) |
|---|---|---|
| `patient.rs` | name + identifiers + addresses for list / detail | **Deleted.** Read path moves to `services/main_person_api.rs`; the template binds against the upstream `Person` shape directly. |
| `ward.rs` | ward + its beds | unchanged |
| `bed.rs` | bed status + current occupant link | reference becomes `person_id`; the template calls `main_person_api.get(person_id)` to render the occupant's name |
| `occupancy.rs` | aggregated bed-status counts per ward (dashboard) | unchanged |
| `rtt.rs` | active pathways + computed weeks-waiting + breach flag | the pathway row carries `person_id`; the template resolves names via `main_person_api` |
| `schedule.rs` | upcoming free slots (14-day horizon) for the booking picker | unchanged |
| `letter_template.rs` | templates for the compose form picker | unchanged |
| `audit.rs` | recent audit rows for the dashboard panel | unchanged (only PAS-owned audit rows surface here; identity-CRUD audit lives on the upstream services) |
| `outbox.rs` | unpublished outbox row count for the dashboard panel | unchanged |

---

## 9. Dependency Stack Specification

Mirrors `Cargo.toml`. Anything below is actually compiled.

| Component | Crate | Purpose |
|---|---|---|
| Full-stack framework | `loco-rs` 0.14 | Brings pinned axum / tower / tower-http / tracing |
| Cookie storage | `axum-extra` 0.10 (`cookie`) | `CookieJar` extractor for CSRF middleware (matches Loco's axum minor) |
| Async runtime | `tokio` 1.49 (`full`) | `#[tokio::main]` in `main.rs` |
| ORM | `sea-orm` 1.1 (`sqlx-postgres`, `runtime-tokio-rustls`, `macros`, `with-chrono`, `with-uuid`, `with-json`) | Read-only projections |
| Migrations (path dep) | `patient-administration-system-migrations` | Schema shared with the PAS Axum binary |
| Serialization | `serde`, `serde_json` 1.0 | JSON encode/decode |
| HTTP client | `reqwest` 0.12 (`rustls-tls`, `json`) | PAS API client (no native-tls; rustls only) |
| Utilities | `uuid` 1.19, `chrono` 0.4 | IDs + timestamps |
| Errors | `thiserror` 2.0 | `PasApiError` enum |
| Async traits | `async-trait` 0.1 | `Hooks` impl |
| Dev — testing | `loco-rs` 0.14 with the `testing` feature | `boot_test::<App>()` helper |
| Dev — serial | `serial_test` 3.2 | `#[serial]` on integration tests so each Loco boot binds the port serially |

**Explicitly not included** (and why):

- `axum`, `tower`, `tower-http`, `tracing` as **direct deps** — Loco brings them; adding a second copy produces conflicting `FromRequest` impls.
- `tera` as a **direct dep** — Loco re-exports it through the `ViewEngine<TeraView>` extractor.
- `argon2`, `jsonwebtoken` — no authentication in v0.1.
- `tantivy`, `utoipa`, `quick-xml`, `csv` — those are PAS-Axum-side concerns.

---

## 10. Configuration Specification

Read from environment (with defaults from `config/<environment>.yaml`).

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `DATABASE_URL` | No (falls back to config) | from `config/<env>.yaml` | PostgreSQL connection string. Same DB as the PAS Axum binary. |
| `PAS_API_URL` | No | `http://localhost:8080` | Base URL for the PAS Axum API — every write flow routes through it |
| `PAS_COOKIE_SECURE` | No | unset (off) | Set to `1` / `true` / `TRUE` / `yes` / `YES` in production so the `pas_csrf` cookie carries `Secure` (HTTPS-only). Off in dev. |
| Loco environment | No | `development` | Standard Loco env switcher (`LOCO_ENV` etc.); pickup of `config/<env>.yaml` |

---

## 11. Testing & Quality Specification

### 11.1 Coverage targets (current baseline = v0.1.0)

| Suite | Count | Notes |
|---|---|---|
| `csrf` unit tests | 9 | constant-time compare, ensure/verify paths, `Secure`-flag env-var toggle |
| Integration smoke tests | 13 | `tests/dashboard_smoke.rs` (gated on `DATABASE_URL`, `#[serial]`) |

### 11.2 Quality gates

Run from the workspace root:

```bash
cargo build  --workspace --all-targets             # zero errors, zero warnings
cargo test   --workspace --lib                     # 497 PAS + 9 frontend = 506
cargo test   --workspace                           # add 62 + 13 integration (needs DATABASE_URL)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt    --check --all
```

### 11.3 Integration-test conventions

- Use Loco's `boot_test::<App>()` to spin up the full app in-process.
- Gate every test on `DATABASE_URL` so `cargo test` works offline (skipped with no failure).
- Apply `#[serial]` (via `serial_test`) so each Loco boot binds the listening port serially — concurrent boots would race for `5150`.
- Assert both status + body shape. For CSRF tests, GET first to capture the cookie, then POST with / without a matching `csrf_token`.

### 11.4 Unit-test conventions

- Place tests in a `#[cfg(test)] mod tests` block at the bottom of the source file.
- Constant-time compare tests cover: equal, unequal-same-length, unequal-different-length, empty.
- CSRF env-var tests save + restore `PAS_COOKIE_SECURE` to stay independent of other concurrent tests (env is process-global).

---

## 12. Build Sequence (Implementation Tasks)

Ordered, layered build sequence to recreate the **v0.1.0** baseline from a clean checkout. Same-layer tasks parallelize. After every layer, run the gates in §11.2.

### Layer F0 — Crate scaffolding (single agent, blocking)

- **F0.1 Cargo manifest.** `Cargo.toml` with `loco-rs = "0.14"`, `sea-orm`, `axum-extra = { version = "0.10", features = ["cookie"] }`, `reqwest`, `serial_test` (dev-dep). Crate-level `#![allow(clippy::result_large_err)]` in `src/lib.rs` and `src/main.rs`. Migration dep by path (`../patient-administration-system-with-rust/migrations`).
- **F0.2 Workspace registration.** Add `patient-administration-system-frontend` as a member in the workspace `../Cargo.toml`.
- **F0.3 Loco config files.** `config/{development,production,test}.yaml` — bind 5150, dev Postgres URL, production env-driven.

### Layer F1 — `Hooks` trait + boot (after F0)

- **F1.1 `src/app.rs`.** Implement every `Hooks` method: `app_name`, `app_version`, `boot`, `routes`, `register_tasks`, `connect_workers`, `truncate`, `seed`. **Truncate + seed are no-ops** — this crate never mutates the shared schema (invariant §5 #2).
- **F1.2 `src/main.rs`.** Single-line `#[tokio::main] async fn main() -> loco_rs::Result<()> { loco_rs::cli::main::<App, Migrator>().await }`.
- **F1.3 `src/lib.rs`.** Module declarations + crate-level lints.

### Layer F2 — Read models (parallel, after F1)

Pure sea-orm projection queries in `src/models/<name>.rs`. Read-only — no `insert` / `update` / `delete` methods.

- **F2.1 Patient projection** — for list + detail.
- **F2.2 Ward + Bed + Occupancy projections** — for ward detail + dashboard.
- **F2.3 RTT projection** — active pathways + computed weeks-waiting.
- **F2.4 Schedule projection** — upcoming free slots (14-day horizon) for booking picker.
- **F2.5 Letter template projection** — for compose form picker.
- **F2.6 Audit + Outbox projections** — for dashboard panels.

### Layer F3 — CSRF middleware (parallel with F2)

- **F3.1 `src/csrf.rs`.** Double-submit-cookie pattern. `ensure_token(jar) -> (String, CookieJar)` reads or mints `pas_csrf` (`SameSite=Strict; HttpOnly; Path=/`, value = `Uuid::new_v4().simple()`). `verify_token(&jar, &form.csrf_token)` does length-checked constant-time XOR. `cookie_secure()` reads `PAS_COOKIE_SECURE` env var (`1` / `true` / `TRUE` / `yes` / `YES` → on). 9 unit tests.

### Layer F4 — PAS API client (parallel with F2/F3)

- **F4.1 `src/services/pas_api.rs`.** `reqwest::Client` with 10 s timeout. Helpers: `admit(patient_id, bed_id)`, `book_slot(slot_id, patient_id)`, `generate_letter(template_id, patient_id, channel, extra)`. Shared `post_envelope` unwraps the PAS `ApiResponse` envelope, surfaces non-2xx with the parsed `error.message`. `PasApiError` enum: `Http(reqwest::Error)`, `Status { status, code, message }`, `MalformedBody(String)`. Base URL via `PAS_API_URL`, default `http://localhost:8080`.

### Layer F5 — Controllers + Tera views (after F2/F3/F4)

One controller file per area, paired with templates under `assets/views/<area>/`.

- **F5.1 `dashboard.rs` + `dashboard/{index,_wards,_outbox,_audit}.html`.** Full page + three HTMX fragment endpoints. `hx-get` + `hx-trigger="load, every 10s"` on each panel. First paint inlines every fragment.
- **F5.2 `patients.rs` + `patients/{index,show}.html`.** List + detail. 404 with Lily-styled empty state on unknown id.
- **F5.3 `wards.rs` + `wards/show.html`.** Beds + current occupants, links to patient pages.
- **F5.4 `rtt.rs` + `rtt/index.html`.** Active pathways sorted worst-breach-first.
- **F5.5 `admissions.rs` + `admissions/new.html`.** GET sets CSRF cookie + renders form; POST verifies token then proxies `POST /api/admissions`.
- **F5.6 `appointments.rs` + `appointments/new.html`.** Same pattern, proxies `POST /api/slots/{id}/book`.
- **F5.7 `letters.rs` + `letters/new.html`.** Same pattern, proxies `POST /api/letters/generate`; renders the generated letter inline on success.
- **F5.8 `health.rs`.** `/_health` JSON probe.
- **F5.9 `layouts/main.html`.** Single Lily-styled `<style>` block.

### Layer F6 — Tests

- **F6.1 csrf unit tests** (inline `#[cfg(test)] mod tests` in `src/csrf.rs`) — 9 tests as in §11.1.
- **F6.2 `tests/dashboard_smoke.rs`** — 13 integration tests using `boot_test::<App>()`, gated on `DATABASE_URL`, `#[serial]`:
  - Dashboard chrome + Lily markup
  - HTMX fragment isolation (each fragment returns just the panel)
  - Patient list, ward detail, RTT cockpit
  - Admit / book / letter form GETs set cookie + hidden field
  - POST without valid `csrf_token` returns 400
  - Unknown-ward 404
  - `/_health` JSON shape

### Layer F7 — Release

- **F7.1 Workspace CHANGELOG entry.** Note the v0.1.0 first release in `../CHANGELOG.md` alongside the PAS Axum entry for the same day.
- **F7.2 Per-crate CHANGELOG.** This crate's `CHANGELOG.md`, format following Keep a Changelog 1.1.0.
- **F7.3 README** with stack, layout, route table, env vars, CSRF design, design-system notes.

---

## 13. Acceptance Criteria

A fresh checkout matches the **v0.1.0** baseline when **all** of these hold:

- `cargo build --workspace --all-targets` → zero errors, zero warnings.
- `cargo test --workspace --lib` → 497 PAS + 9 frontend = **506 passing**.
- `cargo test --workspace` with a live `DATABASE_URL` → **506 lib + 62 PAS integration + 13 frontend integration** tests passing.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --check --all` → clean.
- `cargo run --bin patient-administration-system-frontend -- start` boots on `:5150` against a migrated DB.
- `GET /dashboard` returns 200 + `text/html` with every Lily panel header.
- `GET /admissions/new` sets a `pas_csrf` cookie and embeds the same value as a hidden `csrf_token` form field.
- `POST /admissions/new` without a valid `csrf_token` returns 400 *before* any PAS API call.
- `POST /admissions/new` with a valid `csrf_token` and a reachable PAS Axum API succeeds and the resulting admission is visible via the PAS Axum REST API.
- `GET /_health` returns `{ service: "patient-administration-system-frontend", version: "0.1.0", database: "ok" }`.

---

## 14. Document Map

| Document | Purpose |
|---|---|
| `spec.md` (this file) | Comprehensive spec-driven-development document for the front-end — single source of truth for scope, invariants, architecture, and build sequence. |
| [`README.md`](README.md) | Public surface — stack, layout, route table, env vars, CSRF design, design-system notes, status. |
| [`CHANGELOG.md`](CHANGELOG.md) | Per-release changelog. Format follows Keep a Changelog 1.1.0; SemVer. |
| [`AGENTS.md`](AGENTS.md) | Agent / human orientation for working on this crate. |
| [`AGENTS/index.md`](AGENTS/index.md) | Index of every AGENTS file under this crate. |
| [`../README.md`](../README.md) | Workspace-level README (PAS + frontend + migration crate). |
| [`../CHANGELOG.md`](../CHANGELOG.md) | Workspace-level changelog coordinating member-crate releases. |
| [`../Cargo.toml`](../Cargo.toml) | Workspace manifest (three members, hoisted `[profile.release]` + `[profile.bench]`). |
| [`../patient-administration-system-with-rust/spec.md`](../patient-administration-system-with-rust/spec.md) | The PAS Axum binary's own spec — the system-of-record surface this crate reads from and proxies writes to. |
| [`../patient-administration-system-with-rust/AGENTS/restful.md`](../patient-administration-system-with-rust/AGENTS/restful.md) | REST shape that this front-end's `pas_api` client consumes. |

---

## 15. Maintenance

This spec is a **living document** — keep it aligned with the shipped surface. Update it when:

- A route is added or removed (refresh §7.1 *and* the route table in `README.md`).
- An invariant changes (rare; treat with the same care as a breaking schema change).
- The dependency stack changes (audit removes or adds a crate; refresh §9 and `Cargo.toml`).
- The build sequence changes (a new layer, a task's ordering shifts).
- Test counts change materially (refresh §11.1 and §13).
- The PAS Axum binary's REST envelope or any consumed endpoint changes shape — fix the `pas_api` client first, then this doc.
