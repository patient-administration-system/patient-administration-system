# Changelog

## [Unreleased]

### Changed
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
