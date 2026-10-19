# Testing Strategy and Guide

## Overview

`patient-administration-system-frontend` uses two layers of tests:

- **Unit tests** inline in source files (currently 9 in `src/csrf.rs`).
- **Integration smoke tests** in `tests/dashboard_smoke.rs` (currently 13), gated on `DATABASE_URL` and serialized with `#[serial]` because each Loco boot binds the listening port.

> **v0.2 target (Option C).** The csrf unit-test count stays at 9. The integration tests in `dashboard_smoke.rs` add stub HTTP services for `person-service` + `worker-service` on ephemeral `TcpListener` ports (same pattern the PAS Axum back end uses in its `webhook_test.rs`). The smoke tests still run end-to-end without those upstream services *running on real ports* — the in-process stubs satisfy the new HTTP read paths. Expected integration count stays at 13 (one or two may gain extra assertions for the upstream-resolution path; no new test files are introduced).

## Unit tests

Run with:

```bash
cargo test --lib                    # this crate only
cargo test --workspace --lib        # all three workspace members
```

**Current count: 9 passing tests, 0 failed, 0 ignored.**

All 9 unit tests cover `src/csrf.rs`. See [csrf.md](csrf.md) for the per-test description.

## Integration tests

Run with:

```bash
cargo test                                                # gated on DATABASE_URL; skips when unset
DATABASE_URL=postgres://… cargo test                      # actually exercise them
```

**Current count: 13 integration tests** in `tests/dashboard_smoke.rs`. Every test:

1. Returns early when `DATABASE_URL` is unset (via the shared gate helper) so `cargo test` works offline.
2. Carries `#[serial]` so concurrent boots do not race for port `5150`.
3. Uses Loco's `boot_test::<App>()` to spin up the full app in-process.

Coverage:

| Area | Tests |
|---|---|
| Dashboard chrome + Lily markup | full page contains `header`, `footer`, every `panel` heading |
| HTMX fragment isolation | each `/dashboard/{wards,outbox,audit}` returns just its panel — no `<html>` wrapper |
| Patient list | `GET /patients` returns 200 + Lily `data-table` with rows |
| Ward detail | `GET /wards/{id}` for a known id returns 200; unknown id returns 404 with Lily empty state |
| RTT cockpit | `GET /rtt` returns 200; breach badges present when seeded data triggers them |
| Admit form GET | sets `pas_csrf` cookie + hidden `csrf_token` input |
| Book form GET | same |
| Letter form GET | same |
| Admit form POST without valid `csrf_token` | returns 400 *before* any PAS API call |
| Book form POST without valid `csrf_token` | returns 400 *before* any PAS API call |
| Letter form POST without valid `csrf_token` | returns 400 *before* any PAS API call |
| `/_health` | returns 200 + JSON `{ service, version, database }` |
| Unknown-ward 404 | Lily-styled empty state, not a stack trace |

## `boot_test::<App>()`

Loco's testing helper boots the full app against the configured `DATABASE_URL`, returning a context that exposes a hyper test client. Use it like:

```rust
#[tokio::test]
#[serial]
async fn dashboard_renders() {
    let Some(_) = std::env::var("DATABASE_URL").ok() else { return };
    let boot = boot_test::<App>().await.unwrap();
    let res  = reqwest::get(format!("http://localhost:{}/dashboard", boot.app_context.config.server.port))
        .await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(body.contains("Ward occupancy"));
}
```

`#[serial]` (from `serial_test`) ensures only one test in this binary boots at a time — concurrent boots would race for the configured port.

## Quality gates

Run from the workspace root:

```bash
cargo test --workspace --lib                            # API unit tests plus the front-end's 9 csrf tests
cargo clippy --workspace --all-targets -- -D warnings   # zero warnings
cargo fmt --check --all                                 # formatting clean
cargo build --workspace --all-targets                   # zero errors
```

## Writing new tests

### Unit tests

1. Place in a `#[cfg(test)] mod tests` block at the bottom of the source file.
2. Use descriptive names: `test_<function>_<scenario>`.
3. For env-var-dependent code, save + restore the variable so the test stays independent of other tests in the same binary (env is process-global).

### Integration tests

1. Add to `tests/dashboard_smoke.rs`.
2. Always gate on `DATABASE_URL` — return early when unset so CI without Postgres stays green.
3. Always `#[serial]` — Loco boots bind the listening port.
4. Assert both status and body shape. For CSRF tests, capture the cookie on GET, then assert that POST without it returns 400.

## See Also

- [architecture.md](architecture.md) — layered design the tests exercise
- [routes.md](routes.md) — every route covered by smoke tests
- [csrf.md](csrf.md) — what the 9 unit tests cover
- [../README.md](../README.md#status--v010) — public test-count summary
- [../spec.md](../spec.md#11-testing--quality-specification) — testing spec
