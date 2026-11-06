# Architecture

## System placement

`patient-administration-system-frontend` is one of three workspace members. It is an Axum web app built with [axa](https://github.com/publicprojects-io/axa) deployed on its own port (`5150`) alongside the PAS Axum API server (`8080`). The two share one PostgreSQL database and one migration crate.

> **v0.2 target (Option C).** Patient + practitioner *reads* additionally go through two HTTP services: `person-service` (`:5180`) and `worker-service` (`:5181`). Those services own the identity + workforce data. The shared database loses its `patients` / `practitioners` / `practitioner_roles` / `departments` tables in PAS Axum v0.44.0.

The invariant: **this crate never writes directly to the shared database** and (v0.2+) **never reads patient or practitioner data from sea-orm**. Every write flow proxies through the PAS Axum REST API; identity reads proxy through the upstream services.

## Layered design

```
controller  →  read model (sea-orm)        →  Postgres (read)
            \
             →  pas_api (reqwest)          →  PAS Axum REST API (write)
            /
            CSRF middleware (src/csrf.rs)  guards every POST
```

### controllers (`src/controllers/`)

One file per area: `dashboard`, `patients`, `wards`, `rtt`, `admissions`, `appointments`, `letters`, `health`. Each exports a `routes() -> Routes` builder. `app.rs::routes` aggregates them.

Page handlers use `Extension<Views>` + `State<AppContext>` and return `Result<Response>`:

```rust
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let rows = projections::recent_audit(&ctx.db, 10).await?;
    format::render().view(&v, "dashboard/index.html", json!({ "rows": rows }))
}
```

Write handlers (`admissions::create`, `appointments::create`, `letters::generate`) follow the same shape but additionally:

1. `verify_token(&jar, &form.csrf_token)?` — fails with `Error::BadRequest` (400) on mismatch.
2. Build the request body and call `services::pas_api::*` which speaks to the PAS Axum REST API.
3. On 2xx from PAS, redirect or re-render with the resulting entity. On non-2xx, re-render the form with the parsed `error.message`.

### read models (`src/models/`)

Sea-orm projection queries sized to the templates that consume them. **Read-only** — no `insert` / `update` / `delete` methods on this side of the wall.

| File | Projection of |
|---|---|
| `patient.rs` | name + identifiers + addresses for list / detail |
| `ward.rs` | ward + its beds |
| `bed.rs` | bed status + current occupant link |
| `occupancy.rs` | aggregated bed-status counts per ward (dashboard) |
| `rtt.rs` | active pathways + computed weeks-waiting + breach flag |
| `schedule.rs` | upcoming free slots (14-day horizon) for booking picker |
| `letter_template.rs` | templates for compose form picker |
| `audit.rs` | recent audit rows for dashboard panel |
| `outbox.rs` | unpublished outbox row count for dashboard panel |

### write services (`src/services/pas_api.rs`)

Thin `reqwest::Client` (10 s timeout) that wraps the PAS Axum REST API.

```rust
pub struct PasApi { base_url: String, http: reqwest::Client }

impl PasApi {
    pub async fn admit(&self, patient_id: Uuid, bed_id: Uuid) -> Result<AdmitResponse, PasApiError>;
    pub async fn book_slot(&self, slot_id: Uuid, patient_id: Uuid) -> Result<BookResponse, PasApiError>;
    pub async fn generate_letter(&self, body: GenerateLetter) -> Result<LetterResponse, PasApiError>;
}
```

Shared `post_envelope` unwraps the PAS `ApiResponse` envelope:

- 2xx + `success: true` → `Ok(data)`
- Non-2xx → `Err(PasApiError::Status { status, message })`, with `message` from the RFC 9457 problem the PAS answers (FR-206)
- Transport / JSON-parse failures → `Err(PasApiError::Http(_))` / `Err(PasApiError::MalformedBody(_))`

Base URL via `PAS_API_URL`, default `http://localhost:8080`.

### CSRF middleware (`src/csrf.rs`)

Double-submit-cookie pattern. See [csrf.md](csrf.md) for the threat model + edge cases.

## axa patterns this crate uses

### `src/app.rs`, plain functions

`main` calls `App::run(Cli::parse())`, and that is the whole path: `config` reads the environment, `context` connects, `router` builds the service, and `db` handles the command line. There are no hooks.

| Function | Body |
|---|---|
| `setting` | This service's defaults, where they differ from axa's: the PAS database in development and test, and the pool timeouts. Production has no default `DATABASE_URL`, so it refuses to boot without one |
| `routes` | `AppRoutes::empty()` + every controller's `routes()` |
| `router` | The routes, `/static`, the templates, then axa's middleware around all of them |
| `db` | `migrate` and `status`. `reset`, `down` and `truncate` refuse (FR-205) |

There are no background tasks or workers: the outbox dispatcher lives in the PAS Axum binary. Seeding lives in `pas-seed`.

### `Views`

axa has no template engine, so `src/views/mod.rs` holds a small one: every `assets/views/**/*.html` loads once at boot, so a change to a template needs a restart. Tera escapes HTML by default.

### `Routes::new().prefix("/x").add("/y", get(handler))`

axa's route table. Each controller exports `pub fn routes() -> Routes` so `App::routes` can compose them.

## Dependency-graph hygiene

`axa` comes from crates.io and appears in both `[dependencies]` and `[dev-dependencies]`; keep the two equal, and keep `axum` on axa's minor. `axum-extra` provides one `CookieJar` type to every handler. `tests/dashboard_smoke.rs` implements `TestApp` itself, so axa's `testing` feature stays out of the production build.

## Write-flow proxy contract

Three POST routes constitute the entire write surface. Each one:

1. Renders a Lily-marked-up form on GET, sets the `pas_csrf` cookie, embeds the same value as a hidden `csrf_token` field.
2. On POST, verifies the cookie ↔ form pairing in constant time. Mismatch → **400 before any side effects**.
3. Builds a typed request to the PAS Axum REST API via `pas_api`.
4. On 2xx, redirects (or re-renders with the result for `letters`).
5. On non-2xx, re-renders the form with the parsed `error.message` inline.

| Front-end POST | PAS Axum REST endpoint | Audit + outbox |
|---|---|---|
| `POST /admissions/new` | `POST /api/admissions` | `EncounterAdmitted` |
| `POST /appointments/new` | `POST /api/slots/{id}/book` | `AppointmentBooked` |
| `POST /letters/new` | `POST /api/letters/generate` | `LetterGenerated` (+ optional SMS auto-send audit) |

## See Also

- [routes.md](routes.md) — every route + matching template
- [csrf.md](csrf.md) — CSRF middleware deep-dive
- [testing.md](testing.md) — `boot_test::<App>()` + `#[serial]` patterns
- [../README.md](../README.md) — public stack notes + route table
- [../spec.md](../spec.md) — full specification-driven-development document
- [../../patient-administration-system-with-rust/AGENTS/restful.md](../../patient-administration-system-with-rust/AGENTS/restful.md) — REST envelope + error mapping that `pas_api` consumes
