import { describe, expect, it } from "vitest";
import { THEMES, dataThemeFor, themeFrom } from "#lib/theme.js";

describe("themeFrom (NFR-30)", () => {
  it("reads each choice the picker writes", () => {
    for (const t of THEMES) expect(themeFrom(t)).toBe(t);
  });
  it("follows the device when there is no cookie or an unknown one", () => {
    for (const v of [undefined, null, "", "purple", "Dark", "device"]) expect(themeFrom(v)).toBe("system");
  });
});

describe("dataThemeFor", () => {
  it("pins a chosen scheme and leaves the device in charge otherwise", () => {
    expect(dataThemeFor("light")).toBe("light");
    expect(dataThemeFor("dark")).toBe("dark");
    expect(dataThemeFor("system")).toBe("");
  });
});
