import { describe, expect, it } from "vitest";
import { THEMES, safeBack, themeFrom } from "#lib/theme.js";

describe("themeFrom (NFR-30)", () => {
  it("reads each known choice", () => {
    for (const t of THEMES) expect(themeFrom(t)).toBe(t);
  });
  it("follows the device when there is no cookie or an unknown one", () => {
    expect(themeFrom(undefined)).toBe("device");
    expect(themeFrom(null)).toBe("device");
    expect(themeFrom("")).toBe("device");
    expect(themeFrom("purple")).toBe("device");
    expect(themeFrom("Dark")).toBe("device");
  });
});

describe("safeBack", () => {
  it("keeps a path on this site, with its query", () => {
    expect(safeBack("/cy-001/appointments?from=2026-01-01")).toBe("/cy-001/appointments?from=2026-01-01");
  });
  it("never leaves the site", () => {
    for (const bad of [
      "//evil.example/x",
      "/\\evil.example",
      "https://evil.example/",
      "javascript:alert(1)",
      "",
      null,
      undefined,
    ]) {
      expect(safeBack(bad)).toBe("/");
    }
  });
});
