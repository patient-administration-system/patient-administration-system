# AGENTS

Orientation for AI agents (and humans) working on the Patient Administration System (PAS).

**Current shipped version: v0.49.0** (see `Cargo.toml` and `CHANGELOG.md`). Workspace sibling `patient-administration-system-frontend` is at v0.2.0. v0.44 is the **Option C identity delegation** release: `Patient` rows are owned by `person-service` (`:5180`) and `Practitioner` rows by `worker-service` (`:5181`); PAS holds only opaque `person_id` / `worker_id` references on the rows it still owns (encounters, appointments, beds, billing, coverage, consents, letters, schedules, slots). Integration is HTTP-only (no Cargo dep on the upstream crates).

## Start here for AI agents

Read in this order before touching code:

1. [README.md](README.md) — public surface: features, quick start, full endpoint table, configuration, status.
2. [AGENTS/architecture.md](AGENTS/architecture.md) — layered design, aggregates, transactional outbox, state machines, time handling, soft-delete.
3. [AGENTS/models.md](AGENTS/models.md) — every type under `src/models/` with field lists and method signatures.
4. [AGENTS/restful.md](AGENTS/restful.md) — response envelope, error mapping, auth/CORS, headline request shapes.
5. [AGENTS/interchange.md](AGENTS/interchange.md) — v0.2/v0.3 bulk JSON/XML/TSV/CSV import-export, FHIR R5 read/write Bundles, HL7 v2 ADT over HTTP + MLLP.
6. [AGENTS/testing.md](AGENTS/testing.md) — unit-test layout, integration tests gated on `DATABASE_URL`, benchmark suites.

Ground truth for any claim about behavior:

- `src/api/rest/routes/` — the REST route table, one file per area (`mod.rs` merges them; keep all methods of one path in the same function).
- `src/api/fhir/routes.rs` — the FHIR R5 route table.
- `src/api/rest/handlers.rs` — handler shapes and validation.
- `src/lib.rs` — the module tree.
- `src/models/*.rs` — every domain type.
- `migrations/src/m20260520_000001_init.rs` — the database schema.
- `spec.md` — comprehensive specification-driven-development document (scope, invariants, architecture, build sequence, acceptance criteria). Cross-check when in doubt about *what the system is* or *how it was assembled*.

## Directory

| Document | Description |
|----------|-------------|
| [AGENTS/index.md](AGENTS/index.md) | Index of every AGENTS file |
| [AGENTS/architecture.md](AGENTS/architecture.md) | Layered architecture, transactional outbox, four state machines, time handling, soft-delete |
| [AGENTS/models.md](AGENTS/models.md) | Domain model reference, one section per `src/models/*.rs` file |
| [AGENTS/restful.md](AGENTS/restful.md) | REST envelope, error-to-status mapping, auth/CORS contract, headline request shapes |
| [AGENTS/interchange.md](AGENTS/interchange.md) | v0.2/v0.3 interchange surface: bulk JSON/XML/TSV/CSV, FHIR Bundles, HL7 v2 ADT (HTTP + MLLP + outbound) |
| [AGENTS/matching.md](AGENTS/matching.md) | Scheduling overlap detection, slot/bed booking concurrency, RTT clock arithmetic |
| [AGENTS/testing.md](AGENTS/testing.md) | Unit + integration test layout, benchmark suites, quality gates |

## Shared documents

`AGENTS/share/*.md` are short cross-cutting reference snippets used by the PAS and its sister crates. Audit them against the PAS surface before relying on them.

| Document | Description |
|----------|-------------|
| [AGENTS/share/overview.md](AGENTS/share/overview.md) | PAS feature summary |
| [AGENTS/share/technology.md](AGENTS/share/technology.md) | Technology stack reference |
| [AGENTS/share/auditability.md](AGENTS/share/auditability.md) | Audit logging and event streaming |
| [AGENTS/share/availability.md](AGENTS/share/availability.md) | Availability, scaling, health checks |
| [AGENTS/share/observability.md](AGENTS/share/observability.md) | Tracing, metrics, OpenTelemetry |
| [AGENTS/share/privacy.md](AGENTS/share/privacy.md) | Data masking, GDPR export, consent |
| [AGENTS/share/restful.md](AGENTS/share/restful.md) | REST conventions |
| [AGENTS/share/match-search-merge.md](AGENTS/share/match-search-merge.md) | PAS-applicable subset of MPI match/search/merge guidance |

## Working rules

- The PAS is administrative, not clinical. ADT, scheduling, waitlists, resources, communications, billing. No diagnoses, prescriptions, vitals. **Identity and workforce are also out of scope** as of the v0.44.0 target — they live in `person-service` and `worker-service` respectively, reached via HTTP.
- Money is `rust_decimal::Decimal` + `Iso4217`. Never `f64`.
- Time at the domain layer is `chrono::DateTime<Utc>`. Naive local times are never accepted.
- State changes touching multiple rows run inside one `DatabaseTransaction` and write `audit_log` + `outbox_events` rows in the same transaction. **Cross-service writes (PAS + upstream) are not transactional** — Option C accepts this.
- Soft-delete (`deleted_at`) only on `encounters` and `appointments` (v0.44.0 target — was also on `patients` at v0.43.0). Other PAS-owned tables are append-only or operational. Patient/practitioner soft-delete is the upstream's concern.
- **Never depend on `person-service` or `worker-service` as Cargo deps.** Integration is HTTP-only via `src/upstream/`. Adding the upstream crates as path / git deps would pull their full transitive trees in for no benefit.
- README owns the full endpoint table; AGENTS/restful.md does not duplicate it.
- `seed.md` is a historical problem statement — do not modify.
- `spec.md` is the **comprehensive specification-driven-development document** — scope, invariants, architecture, dependency stack, build sequence, acceptance criteria. Treat it as a living document: keep it aligned with the shipped surface whenever any of those things change. Past release history belongs in `CHANGELOG.md`, not in `spec.md`.
- patient-administration-system-frontend lives in the workspace sibling crate `../patient-administration-system-frontend`. It reads via sea-orm but never writes; every write flow proxies through this Axum API so audit + outbox rows still land here.

## Workspace orientation

This crate is one of three workspace members rooted at `..`:

| Crate / Service                                | Role                                                              | Port  |
|------------------------------------------------|-------------------------------------------------------------------|-------|
| `patient-administration-system` (this one)     | Workflow service. REST + FHIR R5 (the `fhir` crate) + HL7 v2 (the `er7` crate; HTTP + MLLP). Proxies identity + workforce to upstream services. | 8080 |
| `patient-administration-system-migrations`     | sea-orm migration crate, shared by both PAS apps; CLI is `pas-migrate`. | n/a |
| `patient-administration-system-frontend`       | Loco-rs read-mostly UI. Tera + HTMX + Lily Design System.         | 5150  |
| `person-service` *(HTTP dep, external)*   | Person identity, demographics, search, match, merge, FHIR R5 Person. | 5180 |
| `worker-service` *(HTTP dep, external)*   | Worker (= practitioner) identity, demographics, search, match, merge, FHIR R5 Worker. | 5181 |

Run workspace gates from the workspace root: `cargo build --workspace`, `cargo test --workspace --lib`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check --all`.
