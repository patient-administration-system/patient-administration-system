import { describe, expect, it } from "vitest";
import {
  DEFAULT_LOCALE,
  LOCALES,
  isRtl,
  isUnlocalized,
  localeFromAcceptLanguage,
  localeOfPath,
  localizePath,
  normalizeLocale,
  splitPath,
} from "#lib/locales.js";

describe("normalizeLocale", () => {
  it("accepts exact codes, ignoring case and underscores", () => {
    expect(normalizeLocale("CY-001")).toBe("cy-001");
    expect(normalizeLocale("zh_cn")).toBe("zh-cn");
  });
  it("maps a region or legacy code to the supported locale of that language", () => {
    expect(normalizeLocale("es-MX")).toBe("es-001");
    expect(normalizeLocale("en")).toBe("en-001");
    expect(normalizeLocale("de-AT")).toBe("de-001");
  });
  it("refuses what it does not know", () => {
    expect(normalizeLocale("xx")).toBeNull();
    expect(normalizeLocale("")).toBeNull();
    expect(normalizeLocale(null)).toBeNull();
  });
});

describe("paths", () => {
  it("splits a locale prefix from the rest", () => {
    expect(splitPath("/cy-001/my/messages")).toEqual({ locale: "cy-001", rest: "/my/messages" });
    expect(splitPath("/cy-001")).toEqual({ locale: "cy-001", rest: "/" });
    expect(localeOfPath("/fr-001/appointments")).toBe("fr-001");
  });
  it("does not treat a two-letter code as a locale", () => {
    expect(splitPath("/en/appointments")).toEqual({ locale: null, rest: "/en/appointments" });
  });
  it("puts a locale in front of an app path, replacing an existing one", () => {
    expect(localizePath("/appointments?x=1", "cy-001")).toBe("/cy-001/appointments?x=1");
    expect(localizePath("/", "cy-001")).toBe("/cy-001");
    expect(localizePath("/de-001/signin", "fr-001")).toBe("/fr-001/signin");
  });
  it("leaves unlocalized and external paths alone", () => {
    expect(localizePath("/signout", "cy-001")).toBe("/signout");
    expect(localizePath("/signin/oidc/callback", "cy-001")).toBe("/signin/oidc/callback");
    expect(localizePath("//evil.example", "cy-001")).toBe("//evil.example");
    expect(isUnlocalized("/_app/immutable/x.js")).toBe(true);
    expect(isUnlocalized("/appointments")).toBe(false);
  });
});

describe("Accept-Language", () => {
  it("prefers higher q and the nearest supported language", () => {
    expect(localeFromAcceptLanguage("de-DE,de;q=0.9,en;q=0.8")).toBe("de-de");
    expect(localeFromAcceptLanguage("fr;q=0.5, cy;q=0.9")).toBe("cy-001");
    expect(localeFromAcceptLanguage("xx, es-MX")).toBe("es-001");
  });
  it("refuses q=0 and nothing", () => {
    expect(localeFromAcceptLanguage("cy;q=0")).toBeNull();
    expect(localeFromAcceptLanguage(null)).toBeNull();
    expect(localeFromAcceptLanguage("*")).toBeNull();
  });
});

describe("direction and defaults", () => {
  it("only Arabic is right-to-left", () => {
    expect(LOCALES.filter((l) => isRtl(l))).toEqual(["ar-001"]);
  });
  it("the default locale is supported", () => {
    expect(LOCALES).toContain(DEFAULT_LOCALE);
  });
});
