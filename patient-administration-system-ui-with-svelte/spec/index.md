# UI specification

The SvelteKit UI for the Patient Administration System. The cross-subproject contract is in [`../../spec/index.md`](../../spec/index.md); this file records the decisions specific to the UI. When they disagree about the integration contract, the root spec wins.

## Roles and routes

| Route | Role | Data |
|---|---|---|
| `/signin` | anyone | none |
| `/my/appointments`, `/my/messages` | `patient` | Only the record named by the verified `person_id` claim |
| `/appointments`, `/waitlist`, `/schedule`, `/patients` | `staff` or `admin` | PAS API |
| `/admin/reminders` | `admin` | Letter templates |

Every route, form action and endpoint checks its role on the server (`src/lib/server/guard.ts`). The navigation only hides links (`src/lib/access.ts`).

## Components

Lily design system, exact-pinned: `svelte-calendar` (staff calendar, patient agenda), `svelte-kanban-board` (waitlist), `svelte-gantt-chart` (four-week schedule), `svelte-data-grid` (patient search). Their labels come from the catalogue where the widget allows.

- Kanban cards put everything in the title. The board names a cell by its title alone but also renders subtitle and badges, which fails WCAG 2.5.3 (label in name).
- The calendar's view buttons are given a minimum size (WCAG 2.5.8).

## Security

Backend-for-frontend. OIDC code flow with PKCE; ID token verified (RS256, issuer, audience, expiry, nonce); sessions server-side; cookie `__Host-pas_oidc`, httpOnly, Secure, SameSite=Lax; 8 hours maximum, 30 minutes idle. The API bearer token never reaches the browser. A strict CSP (`vite.config.ts`) and the header set in `src/lib/security-headers.ts`.

## Tests

Vitest (`tests/unit`), Playwright with axe and a real Keycloak (`tests/e2e`, one worker), a 390x844 project (`mobile.spec.ts`). See [`../../spec/11-testing-strategy.md`](../../spec/11-testing-strategy.md) and [`../../spec/locales.md`](../../spec/locales.md).

## Not done

Manual accessibility audit; Lily widget label translation; sitemap with `hreflang`; CI (root `tasks.md` REF-1).
