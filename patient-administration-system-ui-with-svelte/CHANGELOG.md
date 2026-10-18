# Changelog

## [Unreleased]

### Added
- Project created: SvelteKit 3, Svelte 5, TypeScript 6, Vite 8, adapter-node, pnpm, Node 26, Vitest, Playwright.
- Keycloak OpenID Connect sign-in (authorization code with PKCE), server-side sessions, httpOnly `__Host-` cookie.
- Patient pages: `/my/appointments`, `/my/messages` (secure portal inbox).
- Staff pages: `/appointments` (calendar), `/waitlist` (kanban), `/schedule` (gantt), `/patients` (data grid).
- Administrator page `/admin/reminders` to edit message templates and their channels.
- Lily components: `svelte-calendar`, `svelte-kanban-board`, `svelte-gantt-chart`, `svelte-data-grid`.
- `Containerfile`.
