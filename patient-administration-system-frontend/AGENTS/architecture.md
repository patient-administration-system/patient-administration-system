# Architecture

## System placement

`patient-administration-system-frontend` is one of three workspace members. It is a Loco-rs web app deployed on its own port (`5150`) alongside the PAS Axum API server (`8080`). The two share one PostgreSQL database and one migration crate.

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

Page handlers use the Loco `ViewEngine<TeraView>` extractor + `State<AppContext>` and return `Result<Response>`:

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
- 2xx + `success: false` (treated as application error) or non-2xx → `Err(PasApiError::Status { status, code, message })`
- Transport / JSON-parse failures → `Err(PasApiError::Http(_))` / `Err(PasApiError::MalformedBody(_))`

Base URL via `PAS_API_URL`, default `http://localhost:8080`.

### CSRF middleware (`src/csrf.rs`)

Double-submit-cookie pattern. See [csrf.md](csrf.md) for the threat model + edge cases.

## Loco patterns this crate uses

### `Hooks` trait — fully implemented in `src/app.rs`

| Hook | Body |
|---|---|
| `app_name` | `env!("CARGO_PKG_NAME")` |
| `app_version` | `env!("CARGO_PKG_VERSION").to_string()` |
| `boot` | Delegates to `create_app::<Self, Migrator>(...)`, threading in the shared `Migrator` from the with-rust project's migration sub-crate |
| `routes` | `AppRoutes::with_default_routes()` + every controller's `routes()` |
| `register_tasks` | **no-op** — no background tasks; the outbox dispatcher lives in the PAS Axum binary |
| `connect_workers` | **no-op** — HTTP-only |
| `truncate` | **no-op** — this crate never mutates the shared schema (would corrupt the system of record) |
| `seed` | **no-op** — seeding lives in `pas-seed` in the with-rust project |

### `ViewEngine<TeraView>` + `format::render()`

Every page handler uses the Loco view engine. Templates live under `assets/views/` and are read at runtime (development) or baked into the binary (production) depending on Loco's `serve_static` settings.

### `Routes::new().prefix("/x").add("/y", get(handler))`

The Loco routing DSL. Each controller exports `pub fn routes() -> Routes` so `app.rs::routes` can compose them.

### Crate-level lint allow

`#![allow(clippy::result_large_err)]` in `src/lib.rs` and `src/main.rs` because `loco_rs::Error` is a wide enum that trips the lint on every handler signature. Without the allow, every handler needs an inline `#[allow]`.

## Dependency-graph hygiene

This crate does **not** declare direct `axum`, `tower`, `tower-http`, or `tracing` dependencies. Loco brings pinned versions; adding a second copy yields two incompatible `axum` versions in the dep graph and conflicting `FromRequest` impls. `axum-extra` is added at the same minor (`0.10`) that Loco's `axum 0.8` brings, so a single `CookieJar` type flows through every handler.

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
