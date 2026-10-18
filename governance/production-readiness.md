# Production readiness (checklist, DRAFT)

Tick an item only with evidence. Unticked means not done, not "not applicable".

## Safety and assurance
- [ ] Clinical Safety Officer named; hazard log scored ([`clinical-safety.md`](clinical-safety.md))
- [ ] DPIA reviewed by a DPO ([`dpia.md`](dpia.md))
- [ ] Penetration test (OWASP Top 10) with no finding at CVSS 7.0 or above
- [ ] Cyber Essentials; Cyber Security Charter for suppliers signed
- [ ] DSPT standards met for the deploying organisation

## Identity and access
- [ ] MFA required for staff and administrators (DTAC-5)
- [ ] Per-user API authentication and a trustworthy audit user id (DTAC-4)
- [ ] Process to prove a patient's identity before linking an account (DTAC-9)
- [ ] `PAS_API_TOKEN` set (the API logs "trusted-caller mode" without it)

## Reliability
- [ ] Backups taken and a restore drill run (see [`../operations/backup-and-restore.md`](../operations/backup-and-restore.md))
- [ ] Alerting on dead-letter events and failed message sends
- [ ] Availability target set and measured

## Messaging
- [ ] Consent, opt-out and quiet hours built and tested (NOT-5)
- [ ] Real carrier integrations tested in a sandbox (NOT-3)
- [ ] Delivery receipts and retries (NOT-4)

## Content
- [ ] Welsh (and any other served language) reviewed by a first-language speaker ([`translation-review.md`](translation-review.md))
- [ ] Accessibility checks done by people, statement published ([`accessibility-statement.md`](accessibility-statement.md))

## Supply chain
- [x] `cargo deny check` passes, each ignore justified (`deny.toml`)
- [ ] CI required on the main branch
- [ ] SBOM published with each release
