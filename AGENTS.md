# AGENTS

Orientation for AI agents and people working in this repository. Read it before changing anything. It is a map and a set of rules; the detail lives in the files it points to. Keep it under 40 KB (`wc -c AGENTS.md`).

## What this is

A Patient Administration System (PAS) for the NHS in Wales: appointments, reminders and replies, waiting lists, transport and virtual-visit requests, and the measurement that shows whether any of it is fair. It is **administrative, not clinical**: no diagnoses, prescriptions or vitals. Identity (`Patient`) and workforce (`Practitioner`) live in two other services (`person-service`, `worker-service`) reached over HTTP; the PAS holds opaque `person_id` / `worker_id` UUIDs.

Current state is generated, not written: [`spec/14-implementation-status.md`](spec/14-implementation-status.md). The live work queue is [`tasks.md`](tasks.md). What is left is mostly people and contracts (clinical safety officer, DPIA sign-off, carriers, penetration test, usability testing, translation review), not code.

## Map

| Path | What |
|---|---|
| `patient-administration-system-with-rust/` | The API (Axum, SeaORM, `src/`, `tests/`, `benches/`, `migrations/` crate). Has its own `AGENTS.md` and `AGENTS/` |
| `patient-administration-system-ui-with-svelte/` | The UI: SvelteKit 3, Svelte 5, Lily Design System, Keycloak sign-in, 10 locales, Playwright + Vitest |
| `patient-administration-system-frontend/` | A Loco server-rendered ops dashboard that runs in parallel to the SvelteKit UI. Has its own `AGENTS.md` |
| `spec/` | The specification (18 chapters). **Single source of truth** for behaviour. FRs are in `06`, NFRs in `07` |
| `tasks.md`, `plan.md` | Work queue and research. `tasks.md` is current; update it with every change |
| `governance/` | Clinical safety, DPIA, data inventory (`personal-data-inventory.csv`), model cards, chaos testing, data routes |
| `operations/` | Deployment, runbook, backup. Every switch an operator must know is in `runbook.md` |
| `podman/` | Postgres 18, Keycloak 26.8, Redis 8 for development; realm in `podman/keycloak/pas-realm.json` |
| `scripts/` | `generate-status.sh`, `generate-llms.py`, `check-links.py`, `dev-auth.sh`, `chaos-podman.sh` |
| `llms.txt`, `llms.json` | Generated index for language models (`python3 scripts/generate-llms.py`) |

## The three-part rule

A behavioural change is one change: **spec edit + code edit + test edit** ([`agents/spec-driven-development.md`](agents/spec-driven-development.md)). New requirement: add an `FR-n` row to `spec/06-functional-requirements.md` naming the test that proves it. Then update `tasks.md`, the personal-data inventory if a table or column is added, and `operations/runbook.md` if an operator must do something.

## Commands

```sh
# Rust (from the repo root). The toolchain is pinned in rust-toolchain.toml; CI uses the same.
cargo fmt --all -- --check                      # from the ROOT: the migrations crate is formatted too
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                          # unit tests; the DB tests are #[ignore]d

# Database tests (need PostgreSQL; run one at a time, they set environment variables)
podman compose -f podman/compose.yml up -d postgres    # PAS_PG_PORT=5442 if 5432 is busy
export DATABASE_URL=postgres://pas:pas_dev_password@localhost:5442/pas
cargo run -p patient-administration-system-migrations --bin pas-migrate -- up
cargo test --workspace -- --ignored --test-threads=1
# Also run them on a freshly created empty database: migrations must build the schema from nothing.

# UI (patient-administration-system-ui-with-svelte)
pnpm install && npm run check && npm test       # svelte-check, Vitest
MOCK_PORT=4191 KEYCLOAK_URL=http://localhost:8190 npx playwright test   # needs Keycloak up (PAS_KEYCLOAK_PORT=8190)

# Docs (after ANY docs, spec or test-file change; CI fails if generated files are stale)
scripts/generate-status.sh && scripts/generate-status.sh --check
python3 scripts/check-links.py
python3 scripts/generate-llms.py
```

Before you push: all of the above is green. **After you push, check `gh run list` and fix a red run before moving on.** A past run of six red pushes was caused by one unformatted file in the migrations crate.

## Rules that are tested, so do not break them

- **Support only (NFR-11).** No score may reduce what a patient is offered, reorder or withhold a slot, overbook, or delay a contact. `tests/cap10_support_test.rs` fails on any column whose name contains `score`, `rank`, `penal`, `suppress`, `deprioriti`, `overbook`, `withhold`, `no_contact`, `do_not_contact`, `skip_contact`, `contact_block`, `demote`, `probab` or `risk`; the only allowed one is `noshow_assessments.probability`, with a written reason.
- **Gated features are inert until a switch AND named people.** The no-show score needs `PAS_NOSHOW_SCORE=true` plus an approved fairness review and an approved clinical-safety review (`POST /api/noshow/reviews`), and each clinic starts in shadow. Staff see a band and reasons, never a percentage, until calibrated. Age is not an input by default.
- **A miss triggers contact, never discharge.** Urgent-pathway services (`follow_up_policies`) hold a cancel for a person.
- **Privacy-safe analytics.** Cells under k are hidden (k never below 5; default 10), with secondary suppression; no identifiers; a per-caller daily query budget. See `governance/analytics-differencing.md` for what is not prevented.
- **Reply links and offers:** single-use hashed tokens; GET reads, POST acts.
- **Consent first:** opt-outs and quiet hours (`PAS_QUIET_HOURS`) are enforced at the one choke point every message passes.
- **Nothing a carrier or the API says reaches an error page.** The UI logs PAS error text on the server and shows its own translated text.
- **Chaos tests** (`governance/chaos-testing.md`) hit every route with hostile input; no panic, no 500, no leak. Seeded: `PAS_CHAOS_SEED`. The API-input test refuses to run on a database not named like `pas_chaos` outside CI.

## Rust conventions

Edition 2024 with let-chains. New tables are created with raw SQL in a migration and read with `Statement` (SeaORM 1.1; SeaORM 2 was trialled and rejected, see DEP-2). Money is `Decimal`, never `f64`. Times are `DateTime<Utc>`. Multi-row changes run in one transaction with the audit row and the outbox row. **Never hold a lock guard across `.await`** (it makes the future `!Send`); clone the `Arc` out in a separate method. Do not slice a `&str` by byte offsets on untrusted text: use `.get(..n)` or check `is_ascii()` (the chaos monkey found a panic this way). Error text that reaches a client must not contain SQL, paths or stack traces.

## UI conventions

All text is a catalogue key in `src/lib/messages/*.json` (10 files; `de-de` equals `de-001`, `zh-cn` equals `zh-001`). Every locale must have exactly the English keys and placeholders (`tests/unit/i18n.test.ts`); a word that really is identical to English goes in that test's allow-list. Translations are written by hand, one locale at a time; Welsh and the rest are first drafts awaiting fluent review (`governance/translation-review.md`). Every new route is added to `tests/e2e/a11y.spec.ts` or its coverage test fails. SvelteKit rejects non-page exports from `+page.server.ts`: put shared constants in `src/lib`. The CSP forbids inline `<style>` and scripts. Pages send `Referrer-Policy: same-origin` (not `no-referrer`, which makes SvelteKit refuse form posts).

## Tests that depend on the clock

Three tests once failed only at certain times of day or on certain dates: quiet hours (21:00 to 08:00 clinic time hold texts), a window centred on "now" that missed 09:00 late in the day, and a date such as "23 Oct" containing the text "3 ". When you write a test with a window, a date or a hold, do not let the result depend on when it runs: set `PAS_QUIET_HOURS=off`, centre windows at noon, assert on structure not on digits. The database clock can be a second ahead of the test's: end default windows a few minutes in the future.

## Outward actions need a person

Do **not** publish to crates.io or a registry, tag, bump a version, change repository settings, or send anything to an external service without an explicit go-ahead in the current conversation (REL-1 is intentionally not done). The release workflow's `workflow_dispatch` dry run builds everything and publishes nothing. Closing a pull request is allowed only when asked. Never send a user's email address to an unrelated service.

## Infrastructure caution

`podman compose up` recreates containers. Pass the same port variables the running stack used (`PAS_PG_PORT`, `PAS_KEYCLOAK_PORT`, `PAS_REDIS_PORT`) or a container is recreated on the default port and may fail to start, taking the database down with it. Check `podman ps` first.

## Where to look

Route table `patient-administration-system-with-rust/src/api/rest/routes.rs`; modules `src/lib.rs`; schema = the migrations in order (`migrations/src/`); operator switches `operations/runbook.md`; what is not done `tasks.md` (open items) and `spec/16-open-questions.md`; per-crate detail `patient-administration-system-with-rust/AGENTS/`.
