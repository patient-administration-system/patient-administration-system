# 11. Testing strategy

| Layer | Tool | Needs | Run |
|---|---|---|---|
| Rust unit | `cargo test` | nothing | every push |
| Rust integration | `cargo test -- --ignored` | PostgreSQL (`DATABASE_URL`) | every push, in the `rust-db` job |
| UI unit | Vitest | nothing | every push |
| UI browser | Playwright + axe | Keycloak (`podman/compose.yml`), mock PAS API | every push |
| Dependencies | `cargo deny check` | nothing | every push |
| Links | `scripts/check-links.py` / lychee | nothing | every push |

Rules: database tests are `#[ignore]`d and fail loudly without a database (NFR-10); a requirement names its test or says "not yet tested" (§6, §7); a bug fix adds the test that would have caught it.

Known gaps are listed in §6 and §7 as "not yet tested".

## Chaos testing

Seeded tests that hit every route with hostile input, run the reminder ladder against a failing carrier, race callers for one slot, and make the API misbehave under the UI. See [`governance/chaos-testing.md`](../governance/chaos-testing.md). FR-153 to FR-156.

