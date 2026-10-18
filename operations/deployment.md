# Deployment

## Pieces

| Piece | Image / command | Port | Needs |
|---|---|---|---|
| PostgreSQL 18 | `docker.io/library/postgres:18` | 5432 | Persistent volume at `/var/lib/postgresql` |
| Keycloak (or any OIDC provider) | `quay.io/keycloak/keycloak:26.0` | 8080 | Its own database |
| PAS API | `patient-administration-system-with-rust/Containerfile` | 8080 | `DATABASE_URL`, migrations applied |
| UI | `patient-administration-system-ui-with-svelte/Containerfile` | 3000 | `PAS_API_URL`, `PAS_API_TOKEN`, `OIDC_*` |
| Operator pages (optional) | `cargo run -p patient-administration-system-frontend` | 5150 | `DATABASE_URL` |

Development: `podman compose -f podman/compose.yml up -d` (ports overridable with `PAS_PG_PORT`, `PAS_KEYCLOAK_PORT`). Its passwords and the realm's users are **development only**. Never use them in production.

## Order

1. Start PostgreSQL. 2. Apply migrations: `pas-migrate up`. 3. Start the API. 4. Start the identity provider and import or create the realm. 5. Start the UI.

## Configuration that matters

| Variable | Where | Effect |
|---|---|---|
| `DATABASE_URL` | API, operator pages | Connection string |
| `API_TOKEN` | API | Shared bearer token. **Unset means trusted-caller mode** (no authentication on the API) |
| `PAS_API_URL`, `PAS_API_TOKEN` | UI | Where the UI reaches the API, and the token it sends |
| `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` | UI | Sign-in |
| `OIDC_SESSION_HOURS`, `OIDC_IDLE_MINUTES` | UI | Session limits (8 h, 30 min) |
| `OIDC_SESSION_STORE=redis`, `REDIS_URL` | UI | Needed with more than one UI replica |
| `PAS_PORTAL_URL` | API | Public address, exposed to templates as `portal_url` |
| `PAS_REMINDERS_ENABLED`, `PAS_REMINDER_LEAD_HOURS` | API | Switch reminders on; lead time (default 24 h) |
| `PAS_EMAIL_PROVIDER`, `PAS_WHATSAPP_PROVIDER`, `PAS_APPLE_MESSAGES_PROVIDER`, `PAS_SMS_PROVIDER` | API | `log` or off. **No real carrier exists yet** |

## TLS

Terminate TLS in front of the UI and send `X-Forwarded-Proto: https` (see `container-server.mjs`); the UI then sets HSTS. Without it the UI treats requests as plain HTTP, and sign-in redirects use `http://`.

## Not ready for production

See [`../governance/production-readiness.md`](../governance/production-readiness.md). Notably: API authentication, MFA, patient identity proofing, consent for messaging, and several documents.
