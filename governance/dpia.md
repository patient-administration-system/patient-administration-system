# Data Protection Impact Assessment (DRAFT, needs a DPO)

Structure follows DTAC C2.2.2. Facts come from the code; judgements are marked **[controller to decide]**. Nothing here is legal advice or has been reviewed.

## 1. The product and how it processes data
Administrative hospital system: admissions, appointments, waiting lists, letters, consent, billing, audit. Patients can sign in to read their own appointments and portal messages. See [`../spec/02-scope.md`](../spec/02-scope.md).

## 2. Data fields
Every column is classified in [`personal-data-inventory.csv`](personal-data-inventory.csv). Demographics and contact details are **not** stored here; they live in `person-service` and are fetched when a message is sent.

## 3. Data flows
[`data-flow.md`](data-flow.md).

## 4. End-user security controls
- Patients and staff sign in through OpenID Connect (code flow with PKCE); the ID token is verified (signature, issuer, audience, expiry, nonce). Sessions are server-side; the cookie is httpOnly and `__Host-` prefixed; 8 hours maximum, 30 minutes idle.
- Roles `patient`, `staff`, `admin` are checked on the server for every page and form action. A patient can read only the record named by their verified `person_id` claim.
- **Gaps:** no MFA is required by the shipped realm (DTAC-5); nothing proves a patient is who the account says (DTAC-9); the API itself uses one shared bearer token (DTAC-4).
- On- and off-boarding: accounts are created and disabled in the identity provider. No process is documented. **[controller to decide]**

## 5. Technical and organisational measures
- In transit: TLS is the deployer's responsibility (the UI sets HSTS on HTTPS requests). At rest: PostgreSQL disk encryption is the deployer's responsibility.
- The UI sends a strict Content-Security-Policy and the standard security headers (tested).
- Audit log written in the same transaction as every change. User id, IP and user agent are recorded **as supplied by the caller** (DTAC-4).
- Rate limiting per client IP.

## 6. Where data is stored and processed
**[controller to decide]**. The software calls no non-UK service at run time unless configured to (optional telemetry export, webhooks, message senders).

## 7. Staff access
The maintainers have no access to any deployment. The deploying organisation's staff access is governed by its roles.

## 8. Controllers and processors
The deploying organisation is the controller. Processors and sub-processors (hosting, message carriers, identity provider) **[controller to decide]**; none is chosen in this repository.

## 9. Retention and disposal
[`retention-schedule.md`](retention-schedule.md). No retention job exists yet.

## 10. Risks and mitigations
| Risk | Mitigation today | Residual |
|---|---|---|
| A message reaches the wrong person (shared or lost phone, stale number) | Real senders not built; plan is to send a portal link, not clinical detail (NFR-13) | High until contact hygiene exists |
| A patient reads another patient's record | Server-side scope from the verified token; tested | Account-to-patient link is manual (DTAC-9) |
| Audit trail names the wrong person | Supplied by caller | Medium (DTAC-4) |
| Personal data in logs or events | Events carry ids; payloads may contain text (inventory: `free-text`) | Medium |
| A risk score disadvantages a group | No score exists; rule NFR-11 forbids reducing support | Must be revisited before any score |
| Messages sent without consent or outside hours | Not built (NFR-15) | High until built |

## 11. Data subject rights
Access: JSON export (`GET /api/patients/{id}/export`). Rectification: upstream service. Erasure and restriction: no procedure. Objection to messaging: not built (NOT-5). **[controller to decide]**

## Sign-off
DPO review: not done. Date: none.
