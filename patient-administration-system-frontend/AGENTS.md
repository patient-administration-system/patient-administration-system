# AGENTS

Orientation for AI agents (and humans) working on `patient-administration-system-frontend` — the Axum and axa read-mostly UI sibling to the PAS Axum API server.

**Current shipped version: v0.2.0** (see `Cargo.toml` and `CHANGELOG.md`). Workspace sibling `patient-administration-system` (the PAS Axum API server, workflow service) is at v0.44.0.

> **v0.2.0 has shipped — Option C identity delegation.** Patient reads moved from database projections to `src/services/person_api.rs` (reqwest against `person-service` `:5180`). Write paths still go through the PAS Axum API at `:8080`. Practitioner reads are unused by this crate so no `worker_api.rs` client was needed. See [spec.md](spec.md) §5 + §8 and the PAS Axum `spec.md` §13.

## Start here for AI agents

Read in this order before touching code:

1. [README.md](README.md) — public surface: stack, layout, route table, env vars, CSRF design, design-system notes, status.
2. [spec.md](spec.md) — comprehensive specification-driven-development document (scope, invariants, architecture, build sequence, acceptance criteria).
3. [AGENTS/architecture.md](AGENTS/architecture.md) — layered design, axa patterns, sqlx read projections, CSRF middleware, PAS API client.
4. [AGENTS/routes.md](AGENTS/routes.md) — every route this crate serves, with the matching template path and (for write routes) the PAS API endpoint each one proxies to.
5. [AGENTS/csrf.md](AGENTS/csrf.md) — the double-submit-cookie design in `src/csrf.rs`, threat model, edge cases.
6. [AGENTS/testing.md](AGENTS/testing.md) — csrf unit tests, integration smoke tests, `boot_test::<App>()`, `#[serial]` discipline.

Ground truth for any claim about behavior:

- `src/app.rs` — configuration, routes and the command line, as plain functions; `routes` aggregates every controller.
- `src/controllers/*.rs` — handler shapes; one file per area.
- `src/csrf.rs` — CSRF middleware (incl. its 9 unit tests).
- `src/services/pas_api.rs` — every PAS Axum REST endpoint this crate calls.
- `src/models/*.rs` — read-only sqlx projections.
- `assets/views/**/*.html` — Tera templates (Lily Design System markup).
- `tests/dashboard_smoke.rs` — integration test surface.
- `spec.md` — comprehensive specification-driven-development document.

## Directory

| Document | Description |
|----------|-------------|
| [AGENTS/index.md](AGENTS/index.md) | Index of every AGENTS file under this crate |
| [AGENTS/architecture.md](AGENTS/architecture.md) | Layered design, axa patterns, sqlx projections, write-flow proxy contract |
| [AGENTS/routes.md](AGENTS/routes.md) | Every route + matching template + (for writes) PAS API endpoint proxied |
| [AGENTS/csrf.md](AGENTS/csrf.md) | Double-submit-cookie design, threat model, `PAS_COOKIE_SECURE` toggle |
| [AGENTS/testing.md](AGENTS/testing.md) | `boot_test::<App>()` + `#[serial]` discipline, csrf unit tests, integration smoke tests |

## Working rules

- **Never write directly to the shared PAS PostgreSQL database.** Every state-changing flow proxies through the PAS Axum REST API via `src/services/pas_api.rs`. This preserves the audit + outbox invariants of the workflow service.
- **(v0.2 target) Never read patient or practitioner data from the database.** Those reads go through `services/main_person_api.rs` + `services/main_worker_api.rs` HTTP clients against the upstream services. The PAS Axum back end is the system of record for everything *except* identity + workforce; identity + workforce live upstream.
- `db reset`, `db down` and `db truncate` **refuse**, and the service never migrates on boot (FR-205). Do not change that: this database is the system of record, and this crate only shares it.
- This crate does **not** declare its own migrations. The migration crate (`patient-administration-system-migrations`) is shared by path and owned by the with-rust project; apply migrations once via `pas-migrate up` from the with-rust project.
- Keep `axum` on the same 0.8 line as `axa`; two copies of `axum` give conflicting `FromRequest` impls.
- Every page handler takes `Extension(v): Extension<Views>` and renders with `v.render("template.html", data)`. Never write HTML by hand.
- Every template uses [Lily Design System](https://github.com/LilyDesignSystem/lily) headless class names. Zero inline styles in templates; styling lives in one `<style>` block in `assets/views/layouts/main.html`.
- Every POST form is CSRF-protected by the double-submit-cookie pattern. GET sets the cookie + hidden field; POST verifies with a constant-time compare. Never add a new POST route without wiring CSRF.
- First paint of every page must work **without JavaScript**. HTMX is a progressive enhancement layered on top.
- `seed.md` does not exist in this crate; seed data lives in the PAS Axum binary's `pas-seed` bin.
- `spec.md` is the **comprehensive specification-driven-development document** — scope, invariants, architecture, dependency stack, build sequence, acceptance criteria. Treat it as a living document: keep it aligned with the shipped surface.

## Workspace orientation

This crate is one of three workspace members rooted at `..`:

| Crate | Role | Port |
|---|---|---|
| `patient-administration-system` | System of record. REST + FHIR R5 + HL7 v2 (HTTP + MLLP). v0.44.0. | 8080 |
| `patient-administration-system-migrations` | SQL migrations under sqlx, shared by both apps; CLI is `pas-migrate`. | n/a |
| `patient-administration-system-frontend` (this crate) | Axum and axa read-mostly UI. Tera + HTMX + Lily Design System. v0.2.0. | 1241 |

Run workspace gates from the workspace root: `cargo build --workspace`, `cargo test --workspace --lib`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check --all`.
