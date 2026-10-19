// NFR-5: every locale has every key (no extras), the same placeholders, and is really translated.
import { describe, expect, it } from "vitest";
import { CATALOGUES } from "#lib/messages/index.js";
import { DEFAULT_LOCALE, LOCALES, isRtl, type Locale } from "#lib/locales.js";
import { interpolate, toMessageFormat, translate } from "#lib/i18n.svelte.js";

const en = CATALOGUES[DEFAULT_LOCALE]!;
const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

// Strings that are legitimately identical to English in a language: product and technical names
// (WhatsApp, Apple Messages, SMS, CSV, Excel) in all of them, plus words that really are the same.
const PROPER = ["channel.whatsapp", "channel.apple_messages", "channel.sms", "calendar.exportCsv", "calendar.exportExcel"];
const SAME_AS_ENGLISH: Partial<Record<Locale, string[]>> = {
  "cy-001": [...PROPER, "calendar.agenda"],
  "de-001": [...PROPER, "col.status", "col.name", "waitlist.patient", "priority.routine", "calendar.agenda", "waits.col.patient"],
  "de-de": [...PROPER, "col.status", "col.name", "waitlist.patient", "priority.routine", "calendar.agenda", "waits.col.patient"],
  "es-001": [...PROPER, "error.title", "calendar.agenda"],
  "fr-001": [
    ...PROPER,
    "nav.patients", "col.date", "waitlist.service", "waitlist.patient", "patients.title",
    "patients.gridLabel", "admin.message", "calendar.agenda", "noshow.points", "waits.col.patient", "waits.col.service",
  ],
  "ar-001": PROPER,
  "hi-001": PROPER,
  "zh-001": PROPER,
  "zh-cn": PROPER,
};

describe("catalogues (NFR-5)", () => {
  it("every supported locale has a catalogue", () => {
    for (const l of LOCALES) expect(CATALOGUES[l], `${l} has no catalogue`).toBeDefined();
  });

  for (const locale of LOCALES.filter((l) => l !== DEFAULT_LOCALE)) {
    describe(locale, () => {
      const c = CATALOGUES[locale] ?? {};
      it("has every English key and no extra", () => {
        expect(Object.keys(c).sort()).toEqual(Object.keys(en).sort());
      });
      it("keeps every placeholder", () => {
        for (const k of Object.keys(en)) expect(placeholders(c[k] ?? ""), `${locale} ${k}`).toEqual(placeholders(en[k]!));
      });
      it("has no empty string", () => {
        for (const k of Object.keys(en)) expect((c[k] ?? "").trim().length, `${locale} ${k}`).toBeGreaterThan(0);
      });
      it("is translated: only allow-listed strings equal English", () => {
        const same = Object.keys(en).filter((k) => c[k] === en[k]);
        expect(same.filter((k) => !(SAME_AS_ENGLISH[locale] ?? []).includes(k)), `${locale} untranslated`).toEqual([]);
      });
    });
  }
});

describe("translate", () => {
  // MessageFormat 2 wraps an inserted value in Unicode bidi isolates, so a service name written in
  // one direction cannot scramble a sentence written in the other (important for Arabic).
  const plain = (s: string) => s.replace(/[\u2066-\u2069]/g, "");
  it("formats placeholders with MessageFormat 2", () => {
    expect(plain(translate("waitlist.nobody", "en-001", { service: "ENT" }))).toBe("Nobody is waiting for ENT.");
    expect(plain(translate("waitlist.nobody", "cy-001", { service: "ENT" }))).toBe("Does neb yn aros am ENT.");
  });
  it("isolates inserted values for bidirectional safety", () => {
    expect(translate("waitlist.nobody", "ar-001", { service: "ENT" })).toContain("\u2068ENT\u2069");
  });
  it("falls back to English for a key the locale lacks, then to the key", () => {
    expect(translate("no.such.key", "cy-001")).toBe("no.such.key");
  });
  it("keeps braces and backslashes in catalogue text literal", () => {
    expect(toMessageFormat("a {b} c")).toBe("{{a {$b} c}}");
    expect(toMessageFormat("{{ x }} \\")).toBe("{{\\{\\{ x \\}\\} \\\\}}");
  });
  it("interpolates plainly, leaving an unknown placeholder as written", () => {
    expect(interpolate("hi {name} {other}", { name: "Ada" })).toBe("hi Ada {other}");
  });
});

describe("direction", () => {
  it("Arabic is right-to-left and nothing else is", () => {
    expect(isRtl("ar-001")).toBe(true);
    expect(isRtl("cy-001")).toBe(false);
    expect(LOCALES.filter(isRtl)).toEqual(["ar-001"]);
  });
});
