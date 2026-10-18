// The locale set and the pure functions over it — no runes, no `$app/*`
// imports — so the server hooks, the universal `reroute` hook, the i18n store
// and unit tests all share one definition.
//
// Locale codes are lowercase and hyphenated ("en-001", "zh-cn"). `-001` is the
// UN M.49 code for "world": a language with no regional variant.
//
// URLs: every app page lives under a locale prefix (`/en-001/appointments`), and the prefix is always
// the full `<language>-<region>` code. There is no two-letter alias: `/en/appointments` is not a
// locale URL, so it is an ordinary unprefixed path and gains the reader's own locale prefix
// like any other (and then finds no page). The routes themselves are not moved: the
// universal `reroute` hook (`src/hooks.ts`) strips the prefix before route matching.

/**
 * Locales for which the UI is translated, sorted alphabetically by code
 * (the LocalePicker shows them in this order). To add one, extend this
 * tuple AND add a matching entry to {@link LOCALE_LABELS} and `STRINGS`
 * (in `i18n.svelte.ts`).
 */
export const LOCALES = [
  "ar-001",
  "cy-001",
  "de-001",
  "de-de",
  "en-001",
  "es-001",
  "fr-001",
  "hi-001",
  "zh-001",
  "zh-cn",
] as const;

/** A supported locale code (one of {@link LOCALES}). */
export type Locale = (typeof LOCALES)[number];

/** Fallback locale for an unknown key, locale, or missing translation. */
export const DEFAULT_LOCALE: Locale = "en-001";

/**
 * Human-readable name for the locale switcher, written in that locale.
 * A `-001` (world) locale is labelled by its language alone; a regional
 * locale reads "<language> - <region>" (never "<language> (<region>)").
 */
export const LOCALE_LABELS: Record<Locale, string> = {
  "ar-001": "العربية",
  "cy-001": "Cymraeg",
  "de-001": "Deutsch",
  "de-de": "Deutsch - Deutschland",
  "en-001": "English",
  "es-001": "Español",
  "fr-001": "Français",
  "hi-001": "हिन्दी",
  "zh-001": "中文",
  "zh-cn": "中文 - 中国",
};

/**
 * Right-to-left locales. The layout mirrors `<html dir>` to `rtl` for
 * these (and `ltr` for everyone else) via {@link isRtl}.
 */
export const RTL_LOCALES = ["ar-001"] as const satisfies readonly Locale[];

/**
 * Normalize raw input to a supported locale, or null if unsupported.
 * Case-insensitive and hyphen/underscore-insensitive. A region or legacy
 * code resolves by primary language (`es-MX`, `en_US`, `zh`, `en` all map
 * to the one supported locale of that language).
 */
export function normalizeLocale(raw: string | null | undefined): Locale | null {
  if (!raw) return null;
  const normalized = raw.trim().replace(/_/g, "-").toLowerCase();
  const exact = LOCALES.find((l) => l === normalized);
  if (exact) return exact;
  const primary = normalized.split("-")[0] ?? "";
  return LOCALES.find((l) => l.split("-")[0] === primary) ?? null;
}

/**
 * Whether `locale` is written right-to-left. Tolerant of region subtags via
 * {@link normalizeLocale}.
 */
export function isRtl(locale: string): boolean {
  const primary = normalizeLocale(locale);

  return (
    primary !== null && (RTL_LOCALES as readonly string[]).includes(primary)
  );
}

/** The first path segment, if it is shaped like a locale. */
const FIRST_SEGMENT = /^\/([a-z]{2,3}(?:-[a-z0-9]{2,3})?)(?=\/|$)/;

/** What a pathname's locale prefix says. */
export interface SplitPath {
  /** The supported locale in the prefix, if any. */
  locale: Locale | null;
  /** The path without the prefix (always starts with `/`). */
  rest: string;
}

/**
 * Split a pathname into its locale prefix and the rest.
 * `/cy-001/my/messages` -> `{ locale: "cy-001", rest: "/my/messages" }`;
 * `/en/appointments` and `/appointments` -> neither (a two-letter code is not a locale).
 */
export function splitPath(pathname: string): SplitPath {
  const m = FIRST_SEGMENT.exec(pathname);
  const seg = m?.[1];
  if (seg) {
    const rest = pathname.slice(seg.length + 1) || "/";
    if ((LOCALES as readonly string[]).includes(seg)) {
      return { locale: seg as Locale, rest };
    }
  }
  return { locale: null, rest: pathname || "/" };
}

/** The locale named by a pathname's prefix, or null. */
export function localeOfPath(pathname: string): Locale | null {
  const s = splitPath(pathname);
  return s.locale;
}

/** Paths that are never localized: the BFF API, framework assets, static files. */
const UNLOCALIZED =
  /^\/(api|_app|assets|\.well-known)(\/|$)|^\/(favicon\.svg|robots\.txt|sitemap\.xml|signout|_health)$|^\/signin\/oidc(\/callback)?$/;

/** Whether `pathname` belongs outside the locale tree. */
export function isUnlocalized(pathname: string): boolean {
  return UNLOCALIZED.test(pathname);
}

/**
 * Put `locale` in front of an app path: `localizePath("/appointments?x=1", "cy-001")`
 * -> `/cy-001/appointments?x=1`; `/` -> `/cy-001`. A path that already carries a locale prefix has it
 * replaced; an external or fragment-only or unlocalized path is returned as is.
 */
export function localizePath(path: string, locale: Locale): string {
  if (!path.startsWith("/") || path.startsWith("//") || isUnlocalized(path)) {
    return path;
  }
  const cut = path.search(/[?#]/);
  const pathname = cut === -1 ? path : path.slice(0, cut);
  const tail = cut === -1 ? "" : path.slice(cut);
  const { rest } = splitPath(pathname);
  // The root is `/<locale>`: SvelteKit does not keep a trailing slash.
  return `/${locale}${rest === "/" ? "" : rest}${tail}`;
}

/**
 * The best supported locale for an `Accept-Language` header, or null.
 * Tags are taken in the order given; q-values below 1 are ordered after
 * higher ones (stable), and `q=0` is refused.
 */
export function localeFromAcceptLanguage(
  header: string | null | undefined,
): Locale | null {
  if (!header) return null;
  const tags = header
    .split(",")
    .map((part, i) => {
      const [tag = "", ...params] = part.trim().split(";");
      const q = params
        .map((p) => /^\s*q=([0-9.]+)\s*$/.exec(p)?.[1])
        .find((v) => v !== undefined);
      return { tag: tag.trim(), q: q === undefined ? 1 : Number(q), i };
    })
    .filter((x) => x.tag && x.tag !== "*" && x.q > 0)
    .sort((a, b) => b.q - a.q || a.i - b.i);
  for (const { tag } of tags) {
    const l = normalizeLocale(tag);
    if (l) return l;
  }
  return null;
}

/**
 * Where a URL that is not in the locale tree should go, or null if it is fine
 * where it is: a path with no locale prefix gains `preferred`'s (temporary). Used by the root layout's `load`, which
 * catches client-side navigations the server never sees.
 */
export function localeRedirect(
  url: { pathname: string; search: string; hash: string },
  preferred: Locale,
): { status: 301 | 307; location: string } | null {
  if (isUnlocalized(url.pathname)) return null;
  const { locale, rest } = splitPath(url.pathname);
  if (locale) return null;
  return {
    status: 307,
    location: `${localizePath(rest, preferred)}${url.search}${url.hash}`,
  };
}
