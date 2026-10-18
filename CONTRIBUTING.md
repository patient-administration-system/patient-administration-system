# Contributing

## The specification decides behaviour

Read [`agents/spec-driven-development.md`](agents/spec-driven-development.md). A behavioural change is **one** change containing the spec edit, the code edit and the test edit. Open a task in [`tasks.md`](tasks.md) rather than silently rewriting a spec or code to match each other.

## Before you push

Rust (from the repository root):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                                   # no database needed
DATABASE_URL=… cargo test --workspace -- --ignored --test-threads=1   # needs PostgreSQL
cargo deny check
```

The UI (`patient-administration-system-ui-with-svelte`):

```sh
pnpm install && pnpm run check && pnpm test
KEYCLOAK_URL=http://localhost:8180 pnpm exec playwright test     # needs podman compose -f podman/compose.yml up -d
```

Documents: `python3 scripts/check-links.py` and `scripts/generate-status.sh --check`.

## Rules that are tests

- Tests that need PostgreSQL are `#[ignore]`d and fail if run without `DATABASE_URL`. Never make a test skip silently.
- Every database column is classified in `governance/personal-data-inventory.csv`; add the row when you add the column.
- Every UI string goes through `t()` and exists in every locale. Locale codes are `<language>-<region>`, lowercase, never a bare two-letter code.
- A new UI route must be added to the accessibility audit.
- No `unsafe` in non-test code.
- A risk score may only add support; it must never reduce a slot or contact (NFR-11).

## Translations

Machine drafts are welcome; they must be marked unreviewed in [`governance/translation-review.md`](governance/translation-review.md). A first-language review is better than a new language.

## Licence

By contributing you agree your work is available under the licences in [`LICENSE.md`](LICENSE.md).
