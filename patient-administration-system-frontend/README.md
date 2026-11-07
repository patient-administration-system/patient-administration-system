# patient-administration-system-frontend

Axum and [axa](https://github.com/publicprojects-io/axa) front-end for the Patient Administration System (PAS).

Stack: **Axum + axa + Tera + HTMX + [Lily Design System](https://github.com/LilyDesignSystem/lily)**.

## What this is

A separate web app — *not* a replacement for the existing PAS Axum API server. The two work together:

| Role | Crate | Port | Owns |
|------|-------|------|------|
| System of record (REST + FHIR + HL7 v2) | `patient-administration-system` (Axum) | `8080` | All writes, audit, outbox dispatcher |
| Human-facing UI | `patient-administration-system-frontend` (Axum and axa) | `5150` | Dashboard, patient pages, server-side Tera, HTMX live-refresh |

Both binaries share:
- The same PostgreSQL database
- The same migration crate (`patient-administration-system-migrations`) — apply migrations once via `pas-migrate up` and both apps see the schema

## Layout

```
patient-administration-system-frontend/
├── Cargo.toml              axa + axum-extra + tera + sqlx + reqwest + …
├── assets/
│   ├── views/
│   │   ├── layouts/main.html
│   │   ├── dashboard/{index,_wards,_outbox,_audit}.html
│   │   ├── patients/{index,show}.html
│   │   ├── wards/show.html
│   │   ├── rtt/index.html
│   │   ├── admissions/new.html
│   │   ├── appointments/new.html
│   │   └── letters/new.html
│   └── static/                   served at /static/...
└── tests/
    └── dashboard_smoke.rs        13 integration tests (gated on DATABASE_URL)
```

## Run

```bash
# 1. Apply migrations once via the existing PAS migration CLI.
cd ../patient-administration-system-with-rust
DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas \
  cargo run --bin pas-migrate -- up

# 2. Boot the front-end.
cd ../patient-administration-system-frontend
DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas \
  cargo run -- start

# Browse to http://localhost:5150/dashboard
```

## Routes

| Method | Path | Description |
|--------|------|-------------|
| GET    | `/_health` | What is running (`{ service, version, database }`); `200` even when the database is down |
| GET    | `/_ping` | Liveness: the process answers, with no database call |
| GET    | `/_readiness` | Readiness: `503` naming `database` while it cannot be reached, so point an orchestrator here |
| GET    | `/dashboard` | Ops dashboard — full page render |
| GET    | `/dashboard/wards` | HTMX fragment: ward occupancy |
| GET    | `/dashboard/outbox` | HTMX fragment: unpublished outbox count |
| GET    | `/dashboard/audit` | HTMX fragment: last 10 audit rows |
| GET    | `/patients` | Patient list |
| GET    | `/patients/{id}` | Patient detail |
| GET    | `/wards/{id}` | Ward detail (beds, occupants) |
| GET    | `/rtt` | RTT cockpit — active pathways with weeks-waiting + breach flags |
| GET    | `/admissions/new` | Admission form (patient + bed pickers) |
| POST   | `/admissions/new` | Submit admission (CSRF-protected; calls `POST /api/admissions`) |
| GET    | `/appointments/new` | Booking form (slot + patient pickers) |
| POST   | `/appointments/new` | Submit booking (CSRF-protected; calls `POST /api/slots/{id}/book`) |
| GET    | `/letters/new` | Letter compose form (template + patient + channel pickers) |
| POST   | `/letters/new` | Generate letter (CSRF-protected; calls `POST /api/letters/generate`) |

All write flows (POST endpoints) go through the PAS Axum API so audit + outbox events land in the system of record. patient-administration-system-frontend never writes directly.

## Environment variables

| Var | Default | Purpose |
|-----|---------|---------|
| `DATABASE_URL` | the local PAS database; required in production | PostgreSQL connection string (same DB as PAS Axum). In production it must set `sslmode` to `require`, `verify-ca` or `verify-full`, unless it is a loopback address or `DB_ALLOW_INSECURE=true` |
| `APP_ENV` | `development` in a debug build, `production` in a release build | `production` turns on HSTS and refuses to boot without `DATABASE_URL` |
| `HTTP_CSP` | the policy in `src/app.rs` | Replaces the `content-security-policy` header. Every environment sends the same one, so a page that breaks under it breaks in development first |
| `PORT` | `5150` | The port to listen on |
| `PAS_API_URL` | `http://localhost:8080` | Base URL for the PAS Axum API — every write flow routes through it |
| `PAS_COOKIE_SECURE` | follows the environment | The `pas_csrf` cookie carries the `Secure` flag (HTTPS-only) everywhere production-like, and not in development or test, so plaintext-HTTP localhost works. `1` / `true` / `yes` turns it on and `0` / `false` / `no` turns it off, whatever the environment. |

Run `patient-administration-system-frontend check` with a deployment's settings before starting it. It prints what the service will use, needs no database, and fails on a setting that makes no sense.

## Security — CSRF on write forms

The three POST routes are protected by a double-submit-cookie pattern in `src/csrf.rs`:

1. On every form `GET`, the handler reads or mints a `pas_csrf` cookie (`SameSite=Strict; HttpOnly; Path=/`, value = 128-bit UUID) and embeds the same token as a hidden `csrf_token` input.
2. On `POST`, the handler compares the form field against the cookie via a constant-time byte comparison. Any mismatch (or missing cookie) returns HTTP 400 *before* any side effects.
3. In production the cookie also carries the `Secure` flag, with no setting needed.

`SameSite=Strict` + `HttpOnly` mean a third-party site can neither read the cookie nor cause it to be sent on a cross-origin POST, so an attacker has no way to supply a matching token.

## Design system

Every template uses Lily Design System headless HTML class names. Lily provides:

- `header` / `footer` for page chrome
- `panel` (with `role="region"`) for dashboard cards
- `data-table` / `data-table-head` / `data-table-body` / `data-table-row` / `data-table-th` / `data-table-td` for tables
- `badge` (with `data-status="ok|warn|error"`) for status indicators
- `alert` (with `role="status"`) for inline diagnostics + empty states
- `code` for inline code

Lily is *headless* — zero CSS — so all the styling lives in `assets/views/layouts/main.html`'s `<style>` block. Swap that block for Tailwind, a Sass build, or any other styling without touching the templates.

## Why a separate crate

- Lets the PAS Axum binary stay focused on the API/integration surface (~85 REST routes, FHIR R5, HL7 v2 over HTTP + MLLP) without growing a templating dependency tree.
- The app can be deployed independently (different replicas, different scaling profile).
- Front-end iteration doesn't risk the integration test gauntlet (`cargo test --lib` in the PAS crate stays at 308 passing).

## Status — v0.1.0

**Workspace gates all green**: `cargo build --workspace --all-targets`, `cargo test --workspace --lib`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check --all`. Written when this crate used Loco 0.14.1 with sea-orm 1.1; it now uses axa (FR-205).

- 20 unit tests + 16 integration smoke tests in `tests/dashboard_smoke.rs` (gated on `DATABASE_URL` so `cargo test` works offline).
- Dep audit clean — no unused direct or dev dependencies.

Patterns the crate uses:
- **`src/app.rs`** holds plain functions: `config`, `context`, `routes`, `router`, `run`. `db reset`, `db down` and `db truncate` refuse, because this crate never changes the shared PAS schema.
- **`Extension(v): Extension<Views>`** on every page handler, and `v.render("template.html", data)` with `data` as `json!({...})`. Tera reads the templates from `assets/views/` at boot.
- **`Routes::new().prefix("/x").add("/y", get(handler))`** for routing; routes are aggregated in `App::routes`.

Not in v0.1.0:
- No login / session middleware. Add via `axum-login` when you need it. CSRF is in place so the foundation for authenticated write flows is ready.
- Static asset folder (`assets/static/`) is empty — drop a favicon / custom CSS file in if you want one.

## See also

- [`../README.md`](../README.md) — workspace-level README.
- [`../CHANGELOG.md`](../CHANGELOG.md) — workspace-level changelog coordinating member-crate releases.
- [`../patient-administration-system-with-rust/AGENTS.md`](../patient-administration-system-with-rust/AGENTS.md) — orientation for the PAS Axum API server; the system-of-record contract (response envelopes, audit log, outbox events) that every write flow here proxies into.
- [`../patient-administration-system-with-rust/AGENTS/restful.md`](../patient-administration-system-with-rust/AGENTS/restful.md) — REST shape that this front-end's reqwest client consumes (`POST /api/admissions`, `POST /api/slots/{id}/book`, `POST /api/letters/generate`).
