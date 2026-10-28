// FR-156: when the PAS API misbehaves, every UI page shows an error page, never the API's text. The browser
// check is `tests/e2e/chaos.spec.ts`; this is the guard that no page escapes it. A new page fails here until
// it is in the chaos list or, with a reason, in KNOWN_GAPS.
import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

/** Page routes, from the route tree: `src/routes/my/messages/+page.svelte` is `/my/messages`. */
function routes(dir = join(process.cwd(), "src/routes"), prefix = ""): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return routes(p, `${prefix}/${name}`);
    return name === "+page.svelte" ? [prefix || "/"] : [];
  });
}

/** The paths `chaos.spec.ts` visits, read from its source so the two cannot drift. */
function chaosPages(): string[] {
  const src = readFileSync(join(process.cwd(), "tests/e2e/chaos.spec.ts"), "utf8");
  const list = /const STAFF_PAGES = \[([\s\S]*?)\];/.exec(src)?.[1] ?? "";
  return [...list.matchAll(/"([^"]+)"/g)].map((m) => m[1].split("?")[0]);
}

// Pages the chaos test does not visit yet. Each is a gap to close, not a decision that it is safe.
const KNOWN_GAPS: Record<string, string> = {
  "/signin": "makes no PAS API call; the OIDC provider is the only thing that can fail",
  "/my/appointments": "patient page; needs a patient session in the chaos run",
  "/my/messages": "patient page; needs a patient session in the chaos run",
  "/r/[token]": "needs a link token in the address; the mock does not mint one for the chaos run",
  "/o/[token]": "needs a link token in the address; the mock does not mint one for the chaos run",
};

describe("every page is chaos-tested or a named gap (FR-156)", () => {
  it("the chaos list names real pages", () => {
    const all = new Set(routes());
    for (const p of chaosPages()) expect(all.has(p), `${p} is not a page`).toBe(true);
  });
  it("a page that is in neither the chaos list nor KNOWN_GAPS fails", () => {
    const covered = new Set(["/", ...chaosPages(), ...Object.keys(KNOWN_GAPS)]);
    expect(routes().filter((r) => !covered.has(r))).toEqual([]);
  });
  it("a known gap is a real page that the chaos list does not already cover", () => {
    const all = new Set(routes());
    const chaos = new Set(chaosPages());
    for (const g of Object.keys(KNOWN_GAPS)) {
      expect(all.has(g), `${g} is not a page`).toBe(true);
      expect(chaos.has(g), `${g} is covered now: remove it from KNOWN_GAPS`).toBe(false);
    }
  });
});
