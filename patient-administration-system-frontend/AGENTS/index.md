# AGENTS Directory Index

Detailed reference documentation for `patient-administration-system-frontend` (Loco-rs UI, **v0.2.0**, paired with PAS Axum v0.44.0).

## Files

| File | Description |
|------|-------------|
| [architecture.md](architecture.md) | Layered design (controller → read model / pas_api → db / PAS Axum), Loco patterns (`Hooks`, `ViewEngine<TeraView>`, `Routes` DSL), sea-orm read projections, CSRF middleware, write-flow proxy contract |
| [routes.md](routes.md) | Every route this crate serves, the template path it renders, and (for write routes) the PAS Axum REST endpoint each one proxies to |
| [csrf.md](csrf.md) | Double-submit-cookie design in `src/csrf.rs` — threat model, cookie attributes, constant-time compare, `PAS_COOKIE_SECURE` toggle |
| [testing.md](testing.md) | 9 csrf unit tests + 13 integration smoke tests in `tests/dashboard_smoke.rs`. `boot_test::<App>()` setup, `#[serial]` discipline, `DATABASE_URL` gating |

## See Also

- [../AGENTS.md](../AGENTS.md) — project-level orientation for agents
- [../README.md](../README.md) — public-facing README with the route table + CSRF design
- [../CHANGELOG.md](../CHANGELOG.md) — per-release changelog (currently v0.2.0)
- [../spec.md](../spec.md) — comprehensive specification-driven-development document
- [../../README.md](../../README.md) — workspace-level README
- [../../CHANGELOG.md](../../CHANGELOG.md) — workspace-level changelog
- [../../Cargo.toml](../../Cargo.toml) — workspace manifest (three members)
- [../../patient-administration-system-with-rust/AGENTS.md](../../patient-administration-system-with-rust/AGENTS.md) — PAS Axum API server orientation; the system-of-record contract (response envelopes, audit log, outbox events) every write flow here proxies into
- [../../patient-administration-system-with-rust/AGENTS/restful.md](../../patient-administration-system-with-rust/AGENTS/restful.md) — REST shape this front-end's `pas_api` client consumes
- [../../patient-administration-system-with-rust/spec.md](../../patient-administration-system-with-rust/spec.md) — PAS Axum spec (system of record)
