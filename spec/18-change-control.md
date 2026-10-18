# 18. Change control

## The rule

A behavioural change is one change: spec + code + test. See [`../agents/spec-driven-development.md`](../agents/spec-driven-development.md).

## Propagation

| If you change | Also update |
|---|---|
| A DTO or response shape in the server | `ui/src/lib/server/pas.ts`, the OpenAPI annotations, `spec/09-api-surface.md`, both CHANGELOGs |
| A table or column | a migration, `spec/10-persistence.md`, `governance/personal-data-inventory.csv` if personal |
| A delivery channel | `DeliveryChannel`, the repository string mapping, `ui/src/lib/channels.ts`, `spec/05-domain-model.md`, the templates editor |
| A locale or string | `spec/locales.md`, the catalogues, the completeness tests |
| A requirement | the test that verifies it, or the words "not yet tested" |
| A dependency | `Cargo.lock` / `pnpm-lock.yaml`, `deny.toml` if an advisory is ignored, CHANGELOG |

## Releases

The tag MUST equal the server crate's version and a CHANGELOG entry MUST exist (`scripts/release-notes.sh`). Versions are not bumped without an explicit decision to publish.
