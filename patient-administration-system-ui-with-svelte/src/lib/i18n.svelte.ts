// i18n: per-locale JSON catalogues (`./messages/<locale>.json`, English is the source of truth)
// formatted by Sveltia I18n (`@sveltia/i18n`, Unicode MessageFormat 2), behind a reactive `$state`
// locale (Svelte 5 runes) and a `t(key, values?)` accessor. The locale set and its pure helpers
// (URL prefixes, Accept-Language) live in `./locales.ts`.
//
// The locale is IN THE URL: every page is served under `/<locale>/…` (`/en-001/appointments`,
// `/cy-001/appointments`). The root layout calls `i18n.set(data.locale)`, so the URL, not
// localStorage, decides what is shown. An unknown key or locale falls back to `en-001`, then to the
// key itself.
//
// Server rendering note: `current` is module-level state shared by every request. That is safe only
// because the layout sets it and the page renders synchronously, with no `await` in between. If
// rendering ever becomes asynchronous, pass the locale explicitly (`translate(key, locale)`).

import { _ as format, addMessages, init as initSveltia, locale as sveltiaLocale } from "@sveltia/i18n";

import { CATALOGUES } from "./messages/index.js";
import {
  DEFAULT_LOCALE,
  LOCALES,
  LOCALE_LABELS,
  RTL_LOCALES,
  isRtl,
  localizePath,
  normalizeLocale,
  type Locale,
} from "./locales.js";

export { DEFAULT_LOCALE, LOCALES, LOCALE_LABELS, RTL_LOCALES, isRtl };
export type { Locale };

/** Values for a message's `{name}` placeholders. */
export type MessageValues = Record<string, string | number | boolean | null | undefined>;

// MessageFormat 2 reads `{`, `}` and `\` in text as syntax. Catalogue text is plain with `{name}`
// placeholders, so convert once at registration: escape the text, turn each `{name}` into
// `{$name}`, and wrap the whole thing as a quoted pattern.
const PLACEHOLDER = /\{([A-Za-z_]\w*)\}/g;

/** One catalogue string as an MF2 message. Exported for the tests. */
export function toMessageFormat(source: string): string {
  let out = "";
  let last = 0;
  const text = (s: string) => s.replace(/\\/g, "\\\\").replace(/[{}]/g, (c) => `\\${c}`);
  for (const m of source.matchAll(PLACEHOLDER)) {
    out += text(source.slice(last, m.index)) + `{$${m[1]}}`;
    last = (m.index ?? 0) + m[0].length;
  }
  return `{{${out}${text(source.slice(last))}}}`;
}

initSveltia({ fallbackLocale: DEFAULT_LOCALE });
for (const locale of LOCALES) {
  const catalogue = CATALOGUES[locale];
  if (!catalogue) continue;
  addMessages(locale, Object.fromEntries(Object.entries(catalogue).map(([k, v]) => [k, toMessageFormat(v)])));
}

let current = $state<Locale>(DEFAULT_LOCALE);

/** Reactive current locale. Reading it (or calling {@link t}) inside a component subscribes it. */
export const i18n = {
  get locale(): Locale {
    return current;
  },
  /** Switch the active locale; unsupported input falls back to the default. */
  set(next: string): void {
    const locale = normalizeLocale(next) ?? DEFAULT_LOCALE;
    current = locale;
    void sveltiaLocale.set(locale);
  },
  get locales(): readonly Locale[] {
    return LOCALES;
  },
};

/** The catalogue text for `key` in `locale`, falling back to English then to the key. */
function raw(key: string, locale: Locale): string {
  return CATALOGUES[locale]?.[key] ?? CATALOGUES[DEFAULT_LOCALE]?.[key] ?? key;
}

/**
 * Translate `key` in `locale` with graceful fallbacks. Pure, so it is unit-testable without a
 * component (it reads reactive state only when `locale` is defaulted).
 */
export function translate(key: string, locale: Locale = current, values?: MessageValues): string {
  if (!CATALOGUES[locale]?.[key]) return interpolate(raw(key, locale), values);
  return format(key, { locale, values: values as Record<string, string | number> });
}

/** Plain `{name}` interpolation for text that has no catalogue entry in the locale. */
export function interpolate(template: string, values?: MessageValues): string {
  if (!values) return template;
  return template.replace(/\{([A-Za-z_]\w*)\}/g, (whole, name: string) =>
    name in values ? String(values[name] ?? "") : whole,
  );
}

/** Reactive translation for components: `t("nav.appointments")`. */
export function t(key: string, values?: MessageValues): string {
  return translate(key, current, values);
}

/** An app path under the current locale: `localeHref("/appointments")` is `/cy-001/appointments`. */
export function localeHref(path: string): string {
  return localizePath(path, current);
}
