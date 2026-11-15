// The person's light or dark choice (NFR-30, UI-4). `device` follows the browser's
// `prefers-color-scheme`; `light` and `dark` pin one. The choice is kept in a cookie the server
// reads, so the first paint is already right and there is no flash of the other theme.

/** The cookie that keeps the choice. Not a session cookie: it outlives sign-out, on purpose. */
export const THEME_COOKIE = "theme";

export const THEMES = ["device", "light", "dark"] as const;
export type Theme = (typeof THEMES)[number];

/** The choice a cookie value names; anything else, including no cookie, follows the device. */
export function themeFrom(value: string | null | undefined): Theme {
  return (THEMES as readonly string[]).includes(value ?? "") ? (value as Theme) : "device";
}

/**
 * Where to go back to after choosing: a path on this site, never another origin. Anything else
 * goes to the home page.
 */
export function safeBack(back: string | null | undefined): string {
  if (!back || !back.startsWith("/") || back.startsWith("//") || back.startsWith("/\\")) return "/";
  return back;
}
