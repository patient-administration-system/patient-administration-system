# Patient Administration System

A Patient Administration System for the NHS in Wales: appointments, reminders and replies, waiting lists, transport and virtual-visit requests, and the measurement that shows whether any of it is fair. Administrative, not clinical.

Identity (`Patient`) and workforce (`Practitioner`) are **delegated over HTTP** to two other services, `person-service` (`:5180`) and `worker-service` (`:5181`); the PAS holds only opaque `person_id` / `worker_id` UUIDs. REST, FHIR R5 and HL7 v2 (HTTP and MLLP) are all supported.

**Status.** Built and tested; not yet in production use. What remains is mostly people and contracts: a Clinical Safety Officer (DCB0129), DPIA sign-off, real carrier contracts, an external penetration test, usability testing with patients and staff, and fluent review of the translations. [`tasks.md`](tasks.md) lists every open item and [`spec/14-implementation-status.md`](spec/14-implementation-status.md) gives the current counts (generated from the repository). Read [`governance/production-readiness.md`](governance/production-readiness.md) before running it for real. Nothing has been published to a registry; releasing is a deliberate step (REL-1).

## What it does

- **Reminders that escalate, safely.** A ladder (7 days, 4 days, 3 days, 24 hours) over text, WhatsApp, email or the portal, in English or Welsh; a patient who stays silent is escalated to a person to phone. Opt-outs, quiet hours and consent are enforced in one place.
- **Replies that act.** Confirm, cancel, ask for another time (real alternatives are offered), or ask for help, by text reply, a single-use link, or a voice keypad; every action is idempotent and audited. A cancel on an urgent pathway is held for a person.
- **Fewer wasted slots.** Released slots are offered to standby patients; cancellations ask for a reason and answer with help that fits.
- **Waiting lists and RTT clocks,** long-wait "still needed?" checks, virtual visits (video or phone, with a phone fallback), transport requests behind a provider interface.
- **Fairness you can measure.** A privacy-safe analytics feed (small cells hidden, query budget), wording experiments, and a postcode-based deprivation lookup the health board supplies.
- **An optional no-show score, support only,** off until named people approve it: a transparent weighted sum, shown to staff as a band with reasons, used only to add a phone call, rolled out one clinic at a time after a shadow period. See [`governance/noshow-model.md`](governance/noshow-model.md).
- **Accessible and multilingual UI.** Ten locales, WCAG 2.2 AA checks in CI, keyboard and 320-pixel reflow tests, sign-in with Keycloak and one-time codes for staff.
- **Tested against misuse.** Seeded chaos tests send hostile input to every route, fail the carrier, race the booking, and make the API misbehave under the UI ([`governance/chaos-testing.md`](governance/chaos-testing.md)).

## Layout

```
.
├── patient-administration-system-with-rust/   the API: Axum, SeaORM; src/, tests/, benches/, migrations/ (own crate)
├── patient-administration-system-ui-with-svelte/  the UI: SvelteKit 3, Svelte 5, Lily Design System; 10 locales
├── patient-administration-system-frontend/    a Loco server-rendered dashboard, parallel to the UI
├── spec/            the specification: functional (FR) and non-functional (NFR) requirements, architecture, status
├── tasks.md, plan.md   the work queue and the research behind it
├── governance/      clinical safety, DPIA, data inventory, model cards, evaluation design, data routes
├── operations/      deployment, runbook (every switch), backup and restore
├── podman/          PostgreSQL 18, Keycloak 26.8 and Redis 8 for development
├── scripts/         generators and checks used by CI
├── AGENTS.md, CLAUDE.md   orientation for AI agents and contributors
└── llms.txt, llms.json    generated index for language models
```

## Run it (development)

Needs the Rust toolchain in `rust-toolchain.toml`, Node 26 with pnpm, and Podman. The full steps, including per-user authentication, are in [`INSTALL.md`](INSTALL.md).

```sh
podman compose -f podman/compose.yml up -d        # PAS_PG_PORT=5442 PAS_KEYCLOAK_PORT=8190 if ports are busy
export DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas
cargo run -p patient-administration-system-migrations --bin pas-migrate -- up
cargo run -p patient-administration-system          # API on :8080, Swagger UI at /swagger-ui/
cd patient-administration-system-ui-with-svelte && pnpm install && pnpm run dev   # UI on :5173
```

Development users in the Keycloak realm `pas`: `patient` / `patient`, `admin` / `admin` (development only).

## Test it

```sh
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                                         # unit tests
cargo test --workspace -- --ignored --test-threads=1           # database tests (DATABASE_URL set)
cd patient-administration-system-ui-with-svelte && npm run check && npm test && npx playwright test
```

[`CONTRIBUTING.md`](CONTRIBUTING.md) explains how a change is made (spec, code and test together) and what CI checks.

## Where to read next

| If you want | Read |
|---|---|
| What the system must do | [`spec/index.md`](spec/index.md) |
| What is built and what is open | [`spec/14-implementation-status.md`](spec/14-implementation-status.md), [`tasks.md`](tasks.md) |
| How to run it in production | [`operations/deployment.md`](operations/deployment.md), [`operations/runbook.md`](operations/runbook.md) |
| Safety, privacy and compliance | [`governance/`](governance/README.md), [`docs/united-kingdom/`](docs/united-kingdom) (NHS DTAC) |
| The API in detail | `patient-administration-system-with-rust/README.md` and its `AGENTS/` |
| Security reporting | [`SECURITY.md`](SECURITY.md) |
| Releases | [`CHANGELOG.md`](CHANGELOG.md) |

## Licence

MIT, Apache-2.0, BSD-3-Clause, GPL-2.0 or GPL-3.0, at your option. See [`LICENSE.md`](LICENSE.md).
