# 7. Non-functional requirements

| ID | Requirement | Verified by |
|---|---|---|
| NFR-1 | Every state-changing call MUST write an `audit_log` row in the same transaction as the change. | **Not yet tested** as a general property |
| NFR-2 | Every state change MUST write an outbox event in the same transaction; failed publishes MUST be retried and dead-lettered, never dropped. | **Not yet tested** as a general property |
| NFR-3 | A patient session MUST be able to read only the record named by the `person_id` claim of its verified ID token, never one named in a URL or form. | `ui/tests/unit/access.test.ts`, `ui/tests/e2e/sign-in.spec.ts` |
| NFR-4 | Locale codes MUST be `<language>-<region>`, lowercase, hyphenated; `-001` means "world"; bare two-letter codes MUST NOT be used. | `ui/tests/unit/locale-names.test.ts` |
| NFR-5 | Every locale MUST have every key, no extra keys, and no visible English outside the translation function. | `ui/tests/unit/i18n.test.ts`, `ui/tests/unit/catalogue-coverage.test.ts` |
| NFR-6 | The UI MUST meet WCAG 2.2 AA on every route (axe), including at a 390x844 viewport. | `ui/tests/e2e/a11y.spec.ts`, `ui/tests/e2e/mobile.spec.ts` (automated checks only; no manual audit yet) |
| NFR-7 | The UI MUST send a Content-Security-Policy and the security-header set, with zero CSP violations. | `ui/tests/e2e/security-headers.spec.ts`, `ui/tests/unit/security-headers.test.ts` |
| NFR-8 | Production code MUST contain no `unsafe`. | the compiler (`#![cfg_attr(not(test), forbid(unsafe_code))]`) |
| NFR-9 | Dependencies MUST pass `cargo deny check`; each advisory ignore MUST state why and what removes it. | CI job `deny` |
| NFR-10 | Tests that need PostgreSQL MUST be `#[ignore]`d and MUST fail, not skip, when run without `DATABASE_URL`. | `tests/common/mod.rs`; CI job `rust-db` |
| NFR-11 | A risk score, when one exists, MUST only add support. It MUST NOT remove, reorder away or withhold an offered slot, overbook, or penalise. | **Not yet tested** (no score exists yet; the test must be written with it) |
| NFR-12 | The personal-data inventory MUST list every column that holds personal data. | `patient-administration-system-with-rust/tests/personal_data_inventory.rs` |
| NFR-13 | Messages on open channels (SMS, WhatsApp, email) SHOULD carry no clinical detail and SHOULD link to the portal instead. | **Not yet tested** (a convention for template authors) |
| NFR-14 | Secrets MUST come from the environment and MUST NOT be logged. | **Not yet tested** |
| NFR-15 | No message MAY be sent on a channel the patient has opted out of, or outside quiet hours. | **Not yet built** (tasks.md NOT-5) |

`ui/` abbreviates `patient-administration-system-ui-with-svelte/`.
| NFR-16 | When per-user authentication is on, the audit user id MUST be the verified token subject; a client-supplied `X-User-Id` MUST be discarded, and `X-On-Behalf-Of` MUST be honoured only from an authenticated service. | `tests/user_auth_test.rs`, `tests/token_issuer_test.rs`, `ui/tests/unit/acting-as.test.ts`, `ui/tests/e2e/sign-in.spec.ts` |
