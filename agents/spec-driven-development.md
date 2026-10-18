# Spec-driven development

The specification in [`spec/`](../spec/index.md) is the canonical artefact. Code conforms to it, not the other way round.

## Authority

- For the **integration contract** between the server, the Loco front-end, the SvelteKit UI and the services they call, `spec/` wins.
- For a **crate's internals**, that crate's own spec wins ([`patient-administration-system-with-rust/spec.md`](../patient-administration-system-with-rust/spec.md), [`patient-administration-system-frontend/spec.md`](../patient-administration-system-frontend/spec.md)).
- When a spec and the code disagree, the spec is right until someone changes it on purpose. Open a task in [`tasks.md`](../tasks.md). Do not silently rewrite the spec to match broken code, or the code to match a spec nobody reviewed.
- When the spec is silent, propose the addition before writing the code.

## Three-part changes

A behavioural change is **one** change containing:

1. the **spec** edit (the requirement, with its ID),
2. the **code** edit,
3. the **test** edit that verifies it.

A pull request missing one of the three is not finished. "I'll write the code now and update the spec later" does not happen.

## Requirements and tests

- Requirements are `NFR-n` (non-functional) and `FR-n` (functional) in [`spec/06-functional-requirements.md`](../spec/06-functional-requirements.md) and [`spec/07-non-functional-requirements.md`](../spec/07-non-functional-requirements.md).
- Each requirement names the test that verifies it, or says **not yet tested**. A test file cites the ID in a comment (`// NFR-3:`).
- A task in `tasks.md` is ticked only when a test or a clearly described manual check confirms its acceptance criterion.

## Style

- Use RFC 2119 words (MUST, SHOULD, MAY) in requirements.
- No code beyond type signatures. Diagrams are Mermaid, never images.
- One concept per section. Split a file near 300 lines.

## Where a change goes

| Change | Edit |
|---|---|
| New or changed endpoint | `spec/09-api-surface.md`, the handler, an integration test |
| New table or column | `spec/10-persistence.md`, a migration, a repository test |
| New personal-data field | `spec/12-compliance.md`, `governance/personal-data-inventory.csv`, and the inventory test fails until both agree |
| New locale or string | `spec/locales.md`, the catalogue, the completeness tests |
| New message channel | `spec/05-domain-model.md`, `spec/06-functional-requirements.md`, `DeliveryChannel`, a delivery test |

## Anti-patterns

- Behaviour that exists only in code.
- A requirement with no test and no "not yet tested" note.
- A test that skips silently when its dependency is missing (database tests are `#[ignore]`d and fail loudly if run without `DATABASE_URL`).
- Editing generated status counts by hand (run `scripts/generate-status.sh`).
