// NFR-4: locale codes are `<language>-<region>`, lowercase and hyphenated. `-001` means "world".
// A bare two-letter code must not appear anywhere: not as a locale, a catalogue file or a label.
import { describe, expect, it } from "vitest";
import { readdirSync } from "node:fs";
import { join } from "node:path";
import { LOCALES, LOCALE_LABELS } from "#lib/locales.js";

const SHAPE = /^[a-z]{2,3}-[a-z0-9]{2,3}$/;

describe("locale codes (NFR-4)", () => {
  it("every supported locale is <language>-<region>, lowercase", () => {
    for (const l of LOCALES) expect(l, l).toMatch(SHAPE);
  });
  it("the locale list is sorted", () => {
    expect([...LOCALES]).toEqual([...LOCALES].sort());
  });
  it("catalogue files are named by supported locale, with no bare two-letter name", () => {
    const files = readdirSync(join(process.cwd(), "src/lib/messages")).filter((f) => f.endsWith(".json"));
    for (const f of files) {
      const code = f.replace(/\.json$/, "");
      expect(code, f).toMatch(SHAPE);
      expect(LOCALES as readonly string[], `${f} is not a supported locale`).toContain(code);
    }
  });
  it("labels are written in the language, language-only for -001 and 'language - region' otherwise", () => {
    for (const l of LOCALES) {
      const label = LOCALE_LABELS[l];
      expect(label.length, l).toBeGreaterThan(0);
      expect(label, l).not.toMatch(/[()]/);
      if (l.endsWith("-001")) expect(label, l).not.toContain(" - ");
      else expect(label, l).toContain(" - ");
    }
  });
});
