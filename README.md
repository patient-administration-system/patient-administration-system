# Patient Administration System

Cargo workspace housing the PAS API server and its Loco-rs front-end.

> **Target architecture: v0.44.0 — Option C identity delegation.**
> Identity (`Patient`) and workforce (`Practitioner`) are HTTP-delegated to
> two external services that ship from separate workspaces:
>
> - `person-service` (`:5180`) — person CRUD / search / match / merge / FHIR Person
> - `worker-service` (`:5181`) — worker (= practitioner) CRUD / search / match / merge / FHIR Worker
>
> The PAS schema drops its `patients` / `practitioners` / `practitioner_roles` /
> `departments` tables; PAS-owned rows reference identity via opaque
> `person_id` / `worker_id` UUID columns. Integration is **HTTP-only** — no
> Cargo dep on the upstream crates. See
> [`patient-administration-system-rust-crate/spec.md`](patient-administration-system-rust-crate/spec.md)
> §13 Layer 11 for the migration plan and [`CHANGELOG.md`](CHANGELOG.md)
> Phase 0 entry for the doc rewrite that landed first.
>
> **Shipped surface: v0.44.0 (PAS Axum) + v0.2.0 (frontend).** Option C
> identity delegation has landed across Phases 0 → 3e (clients, destructive
> migration, REST/FHIR proxies, PAS-owned handlers, HL7 v2 inbound +
> outbound, bulk interchange, FHIR Schedule/Slot writes + Bundle
> Decision #4 enforcement). The Loco-rs frontend ships v0.2.0 paired with
> PAS v0.44 — patient reads moved from a sea-orm projection to an HTTP
> client against `person-service`. Everything below describes that surface.

## Layout

```
.
├── Cargo.toml                                 workspace manifest (3 members + hoisted [profile.release]/[profile.bench])
├── CHANGELOG.md                               workspace-level changelog
├── README.md                                  this file
├── patient-administration-system-rust-crate/  PAS Axum API server (v0.44.0)
│   ├── Cargo.toml
│   ├── src/
│   ├── tests/                                 integration test files
│   ├── benches/                               3 Criterion suites
│   ├── examples/                              cargo example + sample payloads
│   ├── migrations/                            sea-orm migrations crate (`pas-migrate` CLI)
│   ├── AGENTS.md + AGENTS/                    architecture / models / restful / matching / interchange / testing / national-person-identifiers
│   ├── README.md
│   ├── CHANGELOG.md
│   └── spec.md                                comprehensive spec-driven-development document
└── patient-administration-system-frontend/    Loco-rs front-end (v0.2.0)
    ├── Cargo.toml
    ├── src/                                   incl. csrf.rs middleware
    ├── assets/views/                          Tera + Lily + HTMX
    ├── config/                                Loco development/production/test YAML
    ├── tests/                                 dashboard_smoke.rs (13 functions)
    ├── AGENTS.md + AGENTS/                    architecture / routes / csrf / testing
    ├── README.md
    ├── CHANGELOG.md
    └── spec.md                                comprehensive spec-driven-development document
```

## Workspace commands

```bash
cargo build --workspace
cargo test --workspace --lib
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## What each crate does

| Crate | Role | Port | Stack |
|-------|------|------|-------|
| `patient-administration-system` | Workflow service. REST + FHIR R5 + HL7 v2 over HTTP and MLLP. Audit, outbox, scheduling. Identity (`Patient`) + workforce (`Practitioner`) delegated upstream to person-service / worker-service. | `8080` | Axum, sea-orm, utoipa |
| `patient-administration-system-frontend` | Web UI. Server-side rendered Tera with HTMX live-refresh and Lily Design System markup. Read-mostly view layer over the shared database. | `5150` | Loco-rs, Tera, HTMX, Lily |
| `patient-administration-system-migrations` | sea-orm migration crate. Shared between the two app crates. | n/a | sea-orm-migration |

## Quick start

```bash
# 1. Start the database.
cd patient-administration-system-rust-crate
docker compose up -d postgres

# 2. Apply migrations once — both apps see the same schema.
DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas \
    cargo run --bin pas-migrate -- up

# 3. (Optional) Seed demo rows.
DATABASE_URL=… cargo run --bin pas-seed

# 4. Start the PAS Axum API on :8080.
DATABASE_URL=… cargo run --bin patient-administration-system

# 5. In another terminal, start the Loco front-end on :5150.
cd ../patient-administration-system-frontend
DATABASE_URL=… cargo run -- start
```

Then browse to:
- `http://localhost:8080/swagger-ui` — OpenAPI explorer for the API
- `http://localhost:8080/dashboard` — Axum-served ops dashboard
- `http://localhost:5150/dashboard` — Loco-served ops dashboard (parallel)
- `http://localhost:5150/patients` — patient list + detail pages

## See also

- [`CHANGELOG.md`](CHANGELOG.md) — workspace-level release history.
- [`Cargo.toml`](Cargo.toml) — workspace manifest. Lists all three members and hoists `[profile.release]` (LTO + single codegen unit + strip) and `[profile.bench]` (inherits release) so per-member profile blocks (silently ignored by Cargo) are unnecessary.
- [`patient-administration-system-rust-crate/README.md`](patient-administration-system-rust-crate/README.md) — full PAS API documentation, endpoint reference, configuration, status.
- [`patient-administration-system-rust-crate/spec.md`](patient-administration-system-rust-crate/spec.md) — comprehensive spec-driven-development document for the PAS Axum API server.
- [`patient-administration-system-rust-crate/AGENTS.md`](patient-administration-system-rust-crate/AGENTS.md) — agent / human orientation for the PAS Axum crate.
- [`patient-administration-system-rust-crate/CHANGELOG.md`](patient-administration-system-rust-crate/CHANGELOG.md) — PAS Axum release history.
- [`patient-administration-system-frontend/README.md`](patient-administration-system-frontend/README.md) — Loco-rs architecture notes + route table + CSRF design.
- [`patient-administration-system-frontend/spec.md`](patient-administration-system-frontend/spec.md) — comprehensive spec-driven-development document for the Loco-rs front-end.
- [`patient-administration-system-frontend/AGENTS.md`](patient-administration-system-frontend/AGENTS.md) — agent / human orientation for the front-end crate.
- [`patient-administration-system-frontend/CHANGELOG.md`](patient-administration-system-frontend/CHANGELOG.md) — patient-administration-system-frontend release history.

## License

MIT or Apache-2.0 or BSD-3-Clause or GPL-2.0 or GPL-3.0.
