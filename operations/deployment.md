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
| `PAS_PASETO_KEYS_URL` / `PAS_PASETO_KEYS_JSON`, `PAS_PASETO_ISSUER`, `PAS_PASETO_AUDIENCE` | API | Per-user authentication: PASETO v4.public keys (JWKS-style), expected issuer and audience. Keys re-fetched every `PAS_PASETO_KEYS_REFRESH_SECS` (default 300) |
| `PAS_CARRIER_SECRETS` | API | One signing secret per message carrier: `twilio=<16+ chars>,whatsapp=<old>|<new>`. A carrier signs `timestamp.body` with HMAC-SHA256 (`X-Carrier-Id`, `X-Carrier-Timestamp`, `X-Carrier-Signature: sha256=<hex>`); it can reach `/api/write-back/*` only. To rotate, list both secrets with `|`, move the carrier over, then drop the old one |
| `PAS_QUIET_HOURS` | API | Window when text-style reminders wait (clinic time), default `21:00-08:00`; `off` disables. See `governance/messaging-rules.md` |
| `PAS_TIMEZONE`, `PAS_HELP_LINE` | API | Clinic time zone for times in reply texts (default Europe/London) and an optional sentence appended where a person can help ("Call 01234 567890.") |
| `PAS_REQUIRE_AUTH` | API | `true` refuses to start without per-user auth. Set it in every real deployment |
| `PAS_ALLOW_LEGACY_API_TOKEN` | API | `true` keeps the shared token working alongside per-user tokens (migration only) |
| `API_TOKEN` | API | Shared bearer token, used when per-user auth is off. **Unset with no keys means trusted-caller mode** |
| `PAS_API_URL`, `PAS_API_TOKEN` / `PAS_API_TOKEN_FILE` | UI | Where the UI reaches the API, and the token it sends (the file form is re-read, for a rotating service token) |
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

## Issuing API tokens (per-user authentication, DTAC-4)

The API verifies PASETO v4.public tokens; something must issue them. This repository ships a small issuer, `pas-token-issuer` (in the API image as `/usr/local/bin/pas-token-issuer`), for **services**: the UI's server and operator tooling. Staff and patients still sign in through Keycloak; the UI's server then acts for them, and the API records `svc:<service> for <person>`.

```sh
pas-token-issuer keygen --dir /secrets/pas-issuer --kid 2026-10        # once; private.key is 0600
pas-token-issuer serve  --key /secrets/pas-issuer/private.key --kid 2026-10 --sub pas-ui \
  --ttl 300 --token-file /run/pas/service.token --bind 0.0.0.0:8195     # keys at /keys.json
```

Point the API at `http://<issuer>:8195/keys.json` (`PAS_PASETO_KEYS_URL`), set `PAS_REQUIRE_AUTH=true`, and point the UI at the token file (`PAS_API_TOKEN_FILE`). Tokens last five minutes and the file is rewritten every half-TTL.

- **The private key is the crown jewel.** Keep it in your secret store, readable by the issuer only. Anyone with it can act as any user the API knows.
- **The keys URL must be `https`** outside loopback; the API refuses plain `http` otherwise.
- **Rotation:** generate a new key under a new `kid`, publish both public keys (merge the two `keys.json` documents) until the longest token has expired, switch the issuer to the new key, then drop the old one. The API re-reads the keys every `PAS_PASETO_KEYS_REFRESH_SECS`.
- **Per-person tokens** at the API (rather than the UI acting for people) need a full authentication service in place of this issuer. The API does not change: it only reads the key set.
- **Development:** `sh scripts/dev-auth.sh` does all of the above with a throwaway key.
