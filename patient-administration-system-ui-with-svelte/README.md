# Patient Administration System UI (SvelteKit)

The browser front-end for the [Patient Administration System](../README.md): patients sign in to see their own appointments and messages; staff use a calendar, a waitlist board, a schedule chart and a patient grid; administrators edit the reminder message templates.

Same stack as the reference `project-portfolio-management-ui-with-svelte`: SvelteKit 3, Svelte 5 (runes), TypeScript 6 strict, Vite 8, `adapter-node`, pnpm, Node 26, Vitest, Playwright, and the [Lily design system](https://www.npmjs.com/org/lilydesignsystem) (`svelte-calendar`, `svelte-kanban-board`, `svelte-gantt-chart`, `svelte-data-grid`).

## How it fits together

The server side is a backend-for-frontend. The browser never sees a token or the PAS API's bearer token.

```
browser ── cookie ──> SvelteKit server ── bearer ──> PAS REST API
                          │
                          └── OpenID Connect (code + PKCE) ──> Keycloak
```

- **Patients** (`patient` role) can reach `/my/*` only. The person id comes from the verified ID token's `person_id` claim, never from the URL, so one patient cannot ask for another's record.
- **Staff** (`staff`, `admin`) can reach the staff pages. `admin` also reaches `/admin/reminders`.
- Every page and action re-checks its role on the server; the navigation only hides links.

## Run it

```sh
# 1. PostgreSQL and Keycloak (from the repository root)
podman compose -f podman/compose.yml up -d
#    ports busy? PAS_PG_PORT=5442 PAS_KEYCLOAK_PORT=8190 podman compose -f podman/compose.yml up -d

# 2. The PAS API (needs DATABASE_URL; run migrations first with pas-migrate)
# 3. This UI
cp .env.example .env      # set PAS_API_URL, PAS_API_TOKEN, OIDC_ISSUER, ...
pnpm install
pnpm run dev
```

Seeded development users (`podman/keycloak/pas-realm.json`, development only): `patient` / `patient`, `admin` / `admin`. The `patient` user's `person_id` is the nil UUID; set it to a real PAS person id in Keycloak to see real data.

## Test

```sh
pnpm run check          # svelte-check, 0 errors expected
pnpm test               # Vitest
KEYCLOAK_URL=http://localhost:8180 pnpm exec playwright test   # real Keycloak, mock PAS API
```

The Playwright suite builds the app and starts a small mock of the PAS API (`tests/e2e/support/pas-mock.ts`), so no Rust service is needed. It requires Keycloak from step 1.

## Container

```sh
podman build --format docker -f Containerfile -t patient-administration-system-ui .
```

Behind a TLS-terminating proxy, have it send `X-Forwarded-Proto: https`; see `container-server.mjs`.

## Not done yet

Locales and right-to-left layout, an automated accessibility (axe) run, and CI. See [`../tasks.md`](../tasks.md) (REF-5, REF-6, DTAC-12, REF-1).
