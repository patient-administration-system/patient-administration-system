// The person's light or dark choice (NFR-30, UI-4), made with the Lily theme picker in the header.
// `system` follows the browser's `prefers-color-scheme`; `light` and `dark` pin one. The picker keeps
// the choice in a cookie (which the server reads, so the first paint is right) and in localStorage
// (which survives where cookies are cleared); it writes neither until the person chooses.

/** The cookie the picker writes and the server reads. Not a session cookie: it outlives sign-out. */
export const THEME_COOKIE = "theme";
/** The localStorage key the picker writes. */
export const THEME_STORAGE_KEY = "pas-theme";

/** The choices, in menu order; `system` is the picker's `SYSTEM_THEME`. */
export const THEMES = ["system", "light", "dark"] as const;
export type Theme = (typeof THEMES)[number];

/** The themes the picker applies (it puts the slug in `<html data-theme>`). */
export const SCHEMES = ["light", "dark"] as const;

/** The choice a cookie value names. Anything else, including no cookie, follows the device. */
export function themeFrom(value: string | null | undefined): Theme {
  return (THEMES as readonly string[]).includes(value ?? "") ? (value as Theme) : "system";
}

/**
 * The `data-theme` the server renders: the pinned scheme, or empty to leave `prefers-color-scheme`
 * in charge (the theme CSS reads an empty or absent value as "follow the device").
 */
export function dataThemeFor(theme: Theme): string {
  return theme === "system" ? "" : theme;
}
