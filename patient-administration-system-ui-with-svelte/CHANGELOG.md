# Changelog

## [Unreleased]

### Added
- A home page for staff (WPAS-5, FR-219), where staff now land: a tile for each area their role may use, and how many suspected conditions are past their target.
- `/suspicions`: open suspected conditions, longest wait first, filtered by condition, with the subtype (a tumour site, a type of diabetes or dementia) and the target (WPAS-1). The conditions come from the API's active profile (WPAS-8); a code the catalogues do not name is shown by the profile's own name.
- `/enquiry?patient=`: everything the PAS holds for one patient (WPAS-3). The PAS records the read.
- `/wards?ward=`: each bed of a ward and who is in it (WPAS-4).
- The 68 new strings are first drafts in every locale but English.
- `/wards` with no ward chosen lists each facility's active wards to choose from. Its two strings are first drafts in every locale but English.
- Each name in the patient search links to that patient's enquiry.

### Changed
- The request bodies the server side sends to the PAS API are typed from the API contract, so a misspelt or missing field fails `pnpm run check`. A cancelling patient's reason must be one of the PAS's barrier codes (QUAL-15).
- Every server-side call to the PAS API carries the page request's `X-Request-Id`, so a failing page can be followed into the API's logs, and every response says it. An incoming id is kept when it is a safe shape (letters, digits, `.`, `_` and `-`, at most 128) and replaced with a new one otherwise.
- A cut list on `/waits`, on `/tolerances` (the open alerts) and in a patient's contact log on `/history` links to its next page, and a later page links back to the first. The page passes the PAS's signed cursor through its own `?cursor=`; one that is not the shape the PAS writes is never sent, and one the PAS refuses (from before a restart, say) goes back to the first page. The two link texts are first drafts in every locale but English.
- `/tolerances`, `/waits` and a patient's contact log on `/history` say when the list is not complete, and how many it shows (QUAL-49). The text is a first draft in every locale but English.
- `@lilydesignsystem/svelte-kanban-board` 0.8.0 to 0.8.1. A card cell is now named from its content (title, subtitle, badges) instead of an `aria-label` made from the title alone, which fixes WCAG 2.5.3. The waitlist cards keep their subtitle and priority badge again; the workaround that folded everything into the title is removed. `tests/e2e/waitlist.spec.ts` fails on 0.8.0.
- The browser suite's mock API port is configurable (`MOCK_PORT`), because another project on the same machine can hold 4181; one retry in CI.

### Added
- Project created: SvelteKit 3, Svelte 5, TypeScript 6, Vite 8, adapter-node, pnpm, Node 26, Vitest, Playwright.
- Keycloak OpenID Connect sign-in (authorization code with PKCE), server-side sessions, httpOnly `__Host-` cookie.
- Patient pages: `/my/appointments`, `/my/messages` (secure portal inbox).
- Staff pages: `/appointments` (calendar), `/waitlist` (kanban), `/schedule` (gantt), `/patients` (data grid).
- Administrator page `/admin/reminders` to edit message templates and their channels.
- Lily components: `svelte-calendar`, `svelte-kanban-board`, `svelte-gantt-chart`, `svelte-data-grid`.
- `Containerfile`.
