// Every locale's catalogue, loaded eagerly from `./<locale>.json`. A catalogue is a flat object of
// dotted key -> text with `{name}` placeholders. `en-001` is the source of truth: the completeness
// tests require every other locale to have exactly its keys and placeholders.

import { LOCALES, type Locale } from "../locales.js";

const modules = import.meta.glob<Record<string, string>>("./*.json", {
  eager: true,
  import: "default",
});

/** The raw catalogue of each locale, by locale code. A missing file is left out, not faked. */
export const CATALOGUES: Partial<Record<Locale, Record<string, string>>> = {};
export const CATALOGUE_FILES: string[] = [];

for (const [path, catalogue] of Object.entries(modules)) {
  const code = path.replace(/^\.\//, "").replace(/\.json$/, "");
  CATALOGUE_FILES.push(code);
  if ((LOCALES as readonly string[]).includes(code)) CATALOGUES[code as Locale] = catalogue;
}

export type StringKey = string;
