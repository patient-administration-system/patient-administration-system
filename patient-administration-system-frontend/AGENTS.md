# AGENTS

Orientation for AI agents (and humans) working on `patient-administration-system-frontend` — the Loco-rs read-mostly UI sibling to the PAS Axum API server.

**Current shipped version: v0.2.0** (see `Cargo.toml` and `CHANGELOG.md`). Workspace sibling `patient-administration-system` (the PAS Axum API server, workflow service) is at v0.44.0.

> **v0.2.0 has shipped — Option C identity delegation.** Patient reads moved from sea-orm projections to `src/services/person_api.rs` (reqwest against `person-service` `:5180`). Write paths still go through the PAS Axum API at `:8080`. Practitioner reads are unused by this crate so no `worker_api.rs` client was needed. See [spec.md](spec.md) §5 + §8 and the PAS Axum `spec.md` §13.

## Start here for AI agents

Read in this order before touching code:

1. [README.md](README.md) — public surface: stack, layout, route table, env vars, CSRF design, design-system notes, status.
2. [spec.md](spec.md) — comprehensive specification-driven-development document (scope, invariants, architecture, build sequence, acceptance criteria).
3. [AGENTS/architecture.md](AGENTS/architecture.md) — layered design, Loco patterns, sea-orm read projections, CSRF middleware, PAS API client.
4. [AGENTS/routes.md](AGENTS/routes.md) — every route this crate serves, with the matching template path and (for write routes) the PAS API endpoint each one proxies to.
5. [AGENTS/csrf.md](AGENTS/csrf.md) — the double-submit-cookie design in `src/csrf.rs`, threat model, edge cases.
6. [AGENTS/testing.md](AGENTS/testing.md) — csrf unit tests, integration smoke tests, `boot_test::<App>()`, `#[serial]` discipline.

Ground truth for any claim about behavior:

- `src/app.rs` — Loco `Hooks` impl; `routes` aggregates every controller.
- `src/controllers/*.rs` — handler shapes; one file per area.
- `src/csrf.rs` — CSRF middleware (incl. its 9 unit tests).
- `src/services/pas_api.rs` — every PAS Axum REST endpoint this crate calls.
- `src/models/*.rs` — read-only sea-orm projections.
- `assets/views/**/*.html` — Tera templates (Lily Design System markup).
- `tests/dashboard_smoke.rs` — integration test surface.
- `spec.md` — comprehensive specification-driven-development document.

## Directory

| Document | Description |
|----------|-------------|
| [AGENTS/index.md](AGENTS/index.md) | Index of every AGENTS file under this crate |
| [AGENTS/architecture.md](AGENTS/architecture.md) | Layered design, Loco patterns, sea-orm projections, write-flow proxy contract |
| [AGENTS/routes.md](AGENTS/routes.md) | Every route + matching template + (for writes) PAS API endpoint proxied |
| [AGENTS/csrf.md](AGENTS/csrf.md) | Double-submit-cookie design, threat model, `PAS_COOKIE_SECURE` toggle |
| [AGENTS/testing.md](AGENTS/testing.md) | `boot_test::<App>()` + `#[serial]` discipline, csrf unit tests, integration smoke tests |

## Working rules

- **Never write directly to the shared PAS PostgreSQL database.** Every state-changing flow proxies through the PAS Axum REST API via `src/services/pas_api.rs`. This preserves the audit + outbox invariants of the workflow service.
- **(v0.2 target) Never read patient or practitioner data from sea-orm.** Those reads go through `services/main_person_api.rs` + `services/main_worker_api.rs` HTTP clients against the upstream services. The PAS Axum back end is the system of record for everything *except* identity + workforce; identity + workforce live upstream.
- The Loco `truncate` + `seed` hooks are intentionally **no-ops**. Do not implement them — they would corrupt the system of record.
- This crate does **not** declare its own migrations. The migration crate (`patient-administration-system-migrations`) is shared by path and owned by the with-rust project; apply migrations once via `pas-migrate up` from the with-rust project.
- Do not add direct `axum` / `tower` / `tower-http` / `tracing` dependencies. Loco brings pinned versions; adding a second copy produces conflicting `FromRequest` impls.
- Every page handler uses the Loco `ViewEngine<TeraView>` extractor and renders via `format::render().view(&v, "template.html", data)`. Never bypass the engine to write HTML by hand.
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
| `patient-administration-system-migrations` | sea-orm migration crate, shared by both apps; CLI is `pas-migrate`. | n/a |
| `patient-administration-system-frontend` (this crate) | Loco-rs read-mostly UI. Tera + HTMX + Lily Design System. v0.2.0. | 5150 |

Run workspace gates from the workspace root: `cargo build --workspace`, `cargo test --workspace --lib`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check --all`.
