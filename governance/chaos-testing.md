# Chaos testing

Tests that deliberately misuse the PAS and the things it depends on, to find what ordinary tests do not think of. Everything is **seeded**: a failure prints its seed, and the same seed repeats it.

| Layer | What the monkey does | What must hold | Where |
|---|---|---|---|
| API input | Reads the route table and sends every route thousands of hostile requests: random methods, path parameters (`../`, NUL, SQL, template and JNDI strings, emoji, very long), junk query strings, malformed or wrongly-typed JSON | No panic; no 500 (502/503/504 only for an upstream not configured); no stack trace, SQL or file path in a response | `tests/chaos_api_test.rs` (`PAS_CHAOS_SEED`, `PAS_CHAOS_ITERATIONS`, default 2000) |
| Carrier faults | A text carrier that fails or stalls at random while the reminder ladder runs through all four steps for ten silent patients | Nothing sent twice; what the record says was sent is exactly what the carrier delivered; every failure is on the record; every silent patient is still escalated to a person; no booking changed | `tests/chaos_faults_test.rs` |
| Races | 25 callers book the same slot at once, five times | Exactly one wins; one live appointment on the slot | `tests/chaos_faults_test.rs` |
| UI against a misbehaving API | The mock PAS answers with a 500 that leaks internals, garbage, HTML from a proxy, empty bodies, the wrong JSON shape, `null`; or picks one at random per request (`CHAOS_SEED`) | Every staff and patient page shows an error page with a heading; nothing from the API's error text reaches the HTML; no uncaught browser exception; pages recover when the API does | `tests/e2e/chaos.spec.ts` |
| Infrastructure | Stops the database container while a probe runs | The API fails fast and cleanly, then recovers without a restart | `scripts/chaos-podman.sh` (manual, destructive, on a throwaway stack only) |

## Running

```sh
# API input and faults (the API-input test refuses any database not named like `pas_chaos`, outside CI: the monkey writes whatever the API allows)
DATABASE_URL=postgres://pas:...@localhost:5442/pas_chaos \
  PAS_CHAOS_ITERATIONS=20000 cargo test --test chaos_api_test --test chaos_faults_test -- --ignored --test-threads=1 --nocapture
PAS_CHAOS_SEED=2 ...   # repeat a failure
# UI
CHAOS_SEED=7 npx playwright test chaos
```

CI runs the Rust chaos tests with the other database tests (random seed each time, printed on failure) and the UI chaos spec with the browser tests.

## What it has found

- **A panic on any multi-byte text in an HL7 batch** (`POST /api/hl7/v2/batch` with an emoji where a segment name or a date belongs): byte slicing split a character. Fixed in `hl7v2/batch.rs` and `pid_translation.rs` with regression tests.
- **API error text reaching the browser**: the staff pages put the PAS's own error message into the error page payload. Now the message stays in the server log and the page shows its own translated text.
- **Pages that fell over on a wrong-shaped answer** (a string where a list belongs): list endpoints are now checked (`pasArray`) and become a clean 502.

## Known by design, not a bug

- A reminder step whose send fails is recorded as `failed` and is **not retried**; the next step, and the call to a person for a patient who stays silent, are the safety net. The chaos test checks that the net holds. If a retry is wanted, it should be a deliberate change to the ladder (a `failed` step that is retried within its window), with its own tests.

## Not covered yet

- Killing the PAS process mid-write (the database transaction boundaries are tested individually; this would test them together).
- Network partition and latency to person-service, beyond the stubbed failures.
- Clock jumps (the tests use a pretend clock for the ladder, not a skewed database clock).
