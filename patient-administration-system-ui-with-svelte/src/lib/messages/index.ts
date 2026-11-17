// Every locale's catalogue, from `./<locale>.json`. A catalogue is a flat object of dotted key -> text with
// `{name}` placeholders. `en-001` is the source of truth: the completeness tests require every other locale
// to have exactly its keys and placeholders.
//
// English, the fallback, is always loaded. Every other language is loaded when a request needs it
// (`loadCatalogue`): on the server by `hooks.server.ts` before anything renders, and in the browser by the root
// layout's load before the page renders. So a page does not carry all ten languages: they were 70 KB gzipped
// together, and the privacy page's text pushed the shared chunk over the bundle budget (2026-11-17). The tests
// load them all with `loadAllCatalogues`.

import { DEFAULT_LOCALE, LOCALES, type Locale } from "../locales.js";

type Catalogue = Record<string, string>;

const english = import.meta.glob<Catalogue>("./en-001.json", { eager: true, import: "default" });
const later = import.meta.glob<Catalogue>("./*.json", { import: "default" });

/** The raw catalogue of each locale loaded so far, by locale code. A missing file is left out, not faked. */
export const CATALOGUES: Partial<Record<Locale, Catalogue>> = {};
/** Every catalogue file there is, loaded or not. */
export const CATALOGUE_FILES: string[] = Object.keys(later).map((path) =>
  path.replace(/^\.\//, "").replace(/\.json$/, ""),
);

for (const [path, catalogue] of Object.entries(english)) {
  const code = path.replace(/^\.\//, "").replace(/\.json$/, "");
  if ((LOCALES as readonly string[]).includes(code)) CATALOGUES[code as Locale] = catalogue;
}

/**
 * Make sure `locale`'s catalogue is loaded. Returns it when it was loaded just now (so the caller can register
 * it), `null` when it was already there or there is none.
 */
export async function loadCatalogue(locale: Locale): Promise<Catalogue | null> {
  if (CATALOGUES[locale] || locale === DEFAULT_LOCALE) return null;
  const load = later[`./${locale}.json`];
  if (!load) return null;
  const catalogue = await load();
  CATALOGUES[locale] = catalogue;
  return catalogue;
}

/** Load every catalogue (the tests, which check them all). */
export async function loadAllCatalogues(): Promise<void> {
  for (const locale of LOCALES) await loadCatalogue(locale);
}

export type StringKey = string;
