# AGENTS Directory Index

Detailed reference documentation for the Patient Administration System (PAS Axum, **v0.49.0**).

> **v0.44.0 (Option C delegation):** `Patient` and `Practitioner` are HTTP-delegated to `person-service` and `worker-service`. The PAS holds opaque `person_id` / `worker_id` columns. See [`../spec.md`](../spec.md) §13 Layer 11 and the workspace `CHANGELOG.md` Phase 0 entry.

## Files

| File | Description |
|------|-------------|
| [architecture.md](architecture.md) | Layered architecture (api → service → repository → db), aggregates, transactional outbox, four state machines, time handling, soft delete, module structure, error mapping, schema summary |
| [models.md](models.md) | Domain model reference for every type under `src/models/` — fields, factory methods, state-machine methods |
| [restful.md](restful.md) | REST envelope, error-to-status mapping (REST + FHIR), auth/CORS, headline request/response shapes. Full route list lives in `../README.md`. |
| [matching.md](matching.md) | Scheduling and RTT clock reference: appointment overlap detection, slot/bed booking concurrency, RTT clock arithmetic and breach predicate. (Title kept for stable links; content is PAS-specific, not the legacy MPI matching doc.) |
| [testing.md](testing.md) | Testing strategy: unit tests, database integration tests (`#[ignore]`d, gated on `DATABASE_URL`), the chaos tests, three Criterion benches, quality gates. Live counts: `../../spec/14-implementation-status.md` |
| [interchange.md](interchange.md) | v0.2/v0.3 interchange formats: bulk JSON / XML / TSV / CSV import-export, the `PatientRow` projection, FHIR R5 `Bundle` reads + writes, and the HL7 v2 ADT / SIU / DFT / MFN surface (HTTP + MLLP) |
| [national-person-identifiers.md](national-person-identifiers.md) | v0.15 multinational national-government healthcare identifier reference (NHS / NIR / TSI / IHI / HCN) — factory list, system URIs, format rules |

## Shared files

| File | Description |
|------|-------------|
| [share/overview.md](share/overview.md) | PAS feature summary |
| [share/technology.md](share/technology.md) | Technology stack reference (single source of truth lives in the README; this is the snapshot) |
| [share/auditability.md](share/auditability.md) | Audit logging and event streaming surface |
| [share/availability.md](share/availability.md) | Availability, scaling, health checks |
| [share/observability.md](share/observability.md) | Tracing, metrics, OpenTelemetry |
| [share/privacy.md](share/privacy.md) | Data masking, GDPR export, consent |
| [share/restful.md](share/restful.md) | REST conventions |
| [share/match-search-merge.md](share/match-search-merge.md) | PAS-applicable subset of match/search/merge guidance (probabilistic matching lives in the MPI crate, not here) |

## See Also

- [../AGENTS.md](../AGENTS.md) — project-level orientation for agents
- [../README.md](../README.md) — public-facing README with the full endpoint table
- [../CHANGELOG.md](../CHANGELOG.md) — per-release changelog (currently **v0.49.0**)
- [../spec.md](../spec.md) — comprehensive specification-driven-development document (scope, invariants, architecture, dependency stack, build sequence, acceptance criteria)
- [../../README.md](../../README.md) — workspace-level README (PAS + patient-administration-system-frontend + migration crate)
- [../../CHANGELOG.md](../../CHANGELOG.md) — workspace-level changelog that coordinates member-crate releases
- [../../Cargo.toml](../../Cargo.toml) — workspace manifest (three members, hoisted `[profile.release]` + `[profile.bench]`)
- [../../patient-administration-system-frontend/README.md](../../patient-administration-system-frontend/README.md) — Loco-rs front-end sibling crate
- [../../patient-administration-system-frontend/spec.md](../../patient-administration-system-frontend/spec.md) — front-end specification-driven-development document
- [../../patient-administration-system-frontend/AGENTS.md](../../patient-administration-system-frontend/AGENTS.md) — front-end agent orientation
