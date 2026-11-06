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

## Where the checks run

A check is not finished until one command runs it on a developer's machine and the same command runs it in CI. CI is a second runner of the same recipes, not the only place they can run (QUAL-41, `plan.md` §16).

- **Local gate (QUAL-42).** A clean worktree, its own build directory, a fresh database per run, and a refusal when the database clock and the host clock differ by more than 30 seconds. A time-based test with a clock that has drifted fails for a reason that has nothing to do with the code (seen 2026-11-04).
- **Quick hook (QUAL-45).** Format, generated files, links, UI lint: under two minutes, opt-in.
- **Independent check.** The local machine is arm64 macOS; CI is x86_64 Linux. A local pass does not show that the image builds or that an x86_64-only fault is absent. Until a second CI exists (QUAL-44), GitHub's `ci-ok` stays the required, independent check. The local result is a record, and becomes an authority only by a decision (QUAL-43).
- **Public repository.** Fork pull requests are never run on a self-hosted runner.

