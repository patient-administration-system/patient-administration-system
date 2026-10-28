// Every non-English catalogue has a row in the translation-review register, so a locale added without a
// review decision fails here. The register is `governance/translation-review.md`; the rule is in
// `spec/locales.md`. A row is a statement of review status, not a claim that anyone has reviewed it.
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { LOCALES } from "#lib/locales.js";

const register = readFileSync(join(process.cwd(), "..", "governance", "translation-review.md"), "utf8");
const rows = register.split("\n").filter((l) => l.startsWith("| `"));

describe("translation-review register", () => {
  it("has a row for every supported locale except the English source", () => {
    for (const locale of LOCALES.filter((l) => l !== "en-001")) {
      expect(
        rows.some((r) => r.split("|")[1].includes(`\`${locale}\``)),
        `${locale} has no row in governance/translation-review.md`,
      ).toBe(true);
    }
  });
  it("has no row for a locale that is not supported", () => {
    const supported = new Set<string>(LOCALES);
    for (const row of rows) {
      for (const [, code] of row.split("|")[1].matchAll(/`([a-z]{2,3}-[a-z0-9]{2,3})`/g)) {
        expect(supported.has(code), `${code} is in the register but is not a locale`).toBe(true);
      }
    }
  });
  it("has a status on every row, and 'reviewed' only with a reviewer and a date", () => {
    for (const row of rows) {
      const [, , drafted, reviewed, date, status] = row.split("|").map((c) => c.trim());
      expect(status, row).not.toBe("");
      expect(drafted, row).not.toBe("");
      if (/^reviewed/i.test(status)) {
        expect(reviewed, `${row}: reviewed with no reviewer`).not.toBe("");
        expect(date, `${row}: reviewed with no date`).not.toBe("");
      }
    }
  });
});
