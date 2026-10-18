# Patient Administration System: specification

**Source of truth for the cross-subproject contract.** When this spec and a crate spec disagree about crate internals, the crate spec wins; when they disagree about the integration contract, this spec wins. Open a task in [`../tasks.md`](../tasks.md); do not silently rewrite either spec.

**Three-part changes.** A behavioural change is one change: spec edit + code edit + test edit. See [`../agents/spec-driven-development.md`](../agents/spec-driven-development.md).

| § | Chapter |
|---|---|
| 1 | [Purpose and vision](01-purpose-and-vision.md) |
| 2 | [Scope](02-scope.md) |
| 3 | [Stakeholders and users](03-stakeholders-and-users.md) |
| 4 | [Glossary](04-glossary.md) |
| 5 | [Domain model](05-domain-model.md) |
| 6 | [Functional requirements](06-functional-requirements.md) |
| 7 | [Non-functional requirements](07-non-functional-requirements.md) |
| 8 | [Architecture](08-architecture.md) |
| 9 | [API surface](09-api-surface.md) |
| 10 | [Persistence](10-persistence.md) |
| 11 | [Testing strategy](11-testing-strategy.md) |
| 12 | [Compliance](12-compliance.md) |
| 13 | [Tasks](13-tasks.md) |
| 14 | [Implementation status](14-implementation-status.md) |
| 15 | [Roadmap](15-roadmap.md) |
| 16 | [Open questions](16-open-questions.md) |
| 17 | [References](17-references.md) |
| 18 | [Change control](18-change-control.md) |

Cross-cutting chapters: [Locales](locales.md).

Per-subproject detail:

- [`patient-administration-system-with-rust/spec.md`](../patient-administration-system-with-rust/spec.md): the server.
- [`patient-administration-system-frontend/spec.md`](../patient-administration-system-frontend/spec.md): the Loco operator front-end.
- [`patient-administration-system-ui-with-svelte/spec/index.md`](../patient-administration-system-ui-with-svelte/spec/index.md): the SvelteKit UI.
