# Runbook

| Symptom | Likely cause | Check / action |
|---|---|---|
| Sign-in redirects to "not available" | Keycloak down, or `OIDC_ISSUER` wrong | `curl $OIDC_ISSUER/.well-known/openid-configuration` |
| Sign-in loops or fails with "did not complete" | Redirect URI not registered; clock skew; wrong client secret | Match `OIDC_REDIRECT_URI` (default `<origin>/signin/oidc/callback`) to the client; check the UI log for `oidc sign-in refused: <reason>` |
| Redirect URI shows `https://` on a plain-HTTP deployment | Missing `PROTOCOL_HEADER` | Set `PROTOCOL_HEADER=x-forwarded-proto` (the container does) |
| Everyone signed out after a UI restart | In-memory sessions | Use Redis for sessions, or accept it |
| Patient sees "not linked to a patient record" (403) | Account has no valid `person_id` | An administrator sets the `person_id` attribute on the Keycloak account |
| `/api/health` says `database: unreachable` | PostgreSQL down or `DATABASE_URL` wrong | Check the database |
| Events not reaching consumers | Publisher failing | `GET /api/admin/outbox/unpublished`, `GET /api/admin/outbox/dead-letters`; replay with `POST /api/admin/outbox/dead-letters/{id}/replay` |
| No reminders sent | Reminders off, no sender enabled, patient has no contact point, outside the lead window | `PAS_REMINDERS_ENABLED=true`; a channel provider set; check `pas::reminders` log lines |
| Same reminder sent twice | Should not happen (a letter is the marker) | Report it; include the appointment id and the `generated_letters` rows |
| Worker merge webhook returns 500 | Missing column rename | Apply all migrations (`pas-migrate up`) |
| Startup says "trusted-caller mode" | `API_TOKEN` unset | Set it unless a trusted network is the only protection |
| `cargo test` shows tests "ignored" | DB tests need PostgreSQL | `DATABASE_URL=… cargo test -- --ignored` |

## Who to tell

A patient-safety incident follows the deploying organisation's incident process and the Clinical Safety Officer. A suspected vulnerability follows [`../SECURITY.md`](../SECURITY.md).
