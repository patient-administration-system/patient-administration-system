// FR-156: when the PAS API misbehaves, every UI page shows an error page, never the API's text. The browser
// check is `tests/e2e/chaos.spec.ts`; this is the guard that no page escapes it. A new page fails here until
// it is in one of the chaos lists.
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

/** The pages `chaos.spec.ts` visits, read from its lists so the two cannot drift. */
function chaosPages(): string[] {
  const src = readFileSync(join(process.cwd(), "tests/e2e/chaos.spec.ts"), "utf8");
  return ["STAFF_PAGES", "PATIENT_PAGES", "PUBLIC_PAGES"].flatMap((name) => {
    const list = new RegExp(`const ${name} = \\[([\\s\\S]*?)\\];`).exec(src)?.[1];
    expect(list, `chaos.spec.ts has no ${name}`).toBeDefined();
    return [...(list ?? "").matchAll(/"([^"]+)"/g)].map((m) => m[1].split("?")[0]);
  });
}

describe("every page is chaos-tested (FR-156)", () => {
  it("the chaos lists name real pages", () => {
    const all = new Set(routes());
    for (const p of chaosPages()) expect(all.has(p), `${p} is not a page`).toBe(true);
  });
  it("a page that is in none of the chaos lists fails", () => {
    const covered = new Set(["/", ...chaosPages()]);
    expect(routes().filter((r) => !covered.has(r))).toEqual([]);
  });
});
