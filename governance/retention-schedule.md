# Retention schedule (DRAFT)

**The periods below are placeholders, not advice.** The controller must set them from the NHS Records Management Code of Practice (or the equivalent in Wales and Scotland) and its own policy. The software does not delete anything on a schedule yet: there is no retention job.

| Class | Covers | Placeholder | Today |
|---|---|---|---|
| `record` | Encounters, admissions, appointments, waiting list, referrals, consents | Set by the controller | Kept indefinitely; appointments and encounters are soft-deleted (`deleted_at`) |
| `billing` | Accounts, charges, invoices, payments, coverage | Set by the controller | Kept indefinitely |
| `message` | Generated letters and their rendered text | Set by the controller | Kept indefinitely |
| `audit` | `audit_log`, outbox history and dead letters. From NEW-9 the log also holds a `read` row for each successful read of one patient's records (user, address, client, route; never the query string), and an `export_balp` row for each export | Set by the controller; read rows are the larger set and the controller may choose a shorter period for them than for changes | Append-only; never deleted by the application. The controller must decide a period for read rows before go-live (open question) |
| `session` | UI sessions (server memory or Redis) | 8 hours maximum, 30 minutes idle | Enforced (`OIDC_SESSION_HOURS`, `OIDC_IDLE_MINUTES`) |

## Gaps

- No retention job and no secure-deletion procedure (tasks.md, to be added with the DPIA).
- Erasure and rectification requests: the subject-access export exists (`GET /api/patients/{id}/export`); erasure does not, and for health records it is usually refused by law, so the process must be written down.
