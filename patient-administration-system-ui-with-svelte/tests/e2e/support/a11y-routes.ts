// The one inventory of pages that the desktop and phone-width axe suites must audit. Paths with a token or a
// useful query are materialised here, while `ROUTE_PATTERNS` keeps the route-tree coverage check readable.
export const PUBLIC_ROUTES = ["/signin", "/privacy"];
export const LINK_ROUTES = [`/r/${"a".repeat(64)}`, `/o/${"a".repeat(64)}`];
export const PATIENT_ROUTES = ["/my/appointments", "/my/messages"];
export const STAFF_AND_ADMIN_ROUTES = [
  "/appointments",
  "/waitlist",
  "/schedule",
  "/patients?q=ada",
  "/admin/reminders",
  "/history",
  "/waits",
  "/tolerances",
  "/work",
  "/home",
  "/suspicions",
  "/enquiry?patient=00000000-0000-0000-0000-000000000000",
  "/wards?ward=77777777-7777-7777-7777-777777777777",
];

export const ROUTE_PATTERNS = [
  ...PUBLIC_ROUTES,
  "/r/[token]",
  "/o/[token]",
  ...PATIENT_ROUTES,
  ...STAFF_AND_ADMIN_ROUTES,
];

export const AXE_TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
