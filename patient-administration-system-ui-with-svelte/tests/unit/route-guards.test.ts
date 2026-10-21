// @vitest-environment node
// A guard nobody can forget (QUAL-21). Every page that is not deliberately public has a `+page.server.ts`, and
// its `load` and every one of its actions call a guard from `#lib/server/guard` (requireStaff, requireAdmin,
// requirePatient, requireProofedPatient). A new route without one fails here, before review.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const ROUTES = join(process.cwd(), "src/routes");

/** Routes anyone may open, and why. Adding one here is a decision a reviewer must see. */
const PUBLIC: Record<string, string> = {
  "": "the root only redirects each person to the first page their role may use",
  signin: "signing in",
  "signin/oidc": "the start of the OpenID Connect flow",
  "signin/oidc/callback": "the provider's redirect back",
  signout: "signing out",
  "r/[token]": "a reminder's reply link: the token is the credential (CAP-13)",
  "o/[token]": "an offer of a freed appointment: the token is the credential (PILOT-8)",
};

const GUARD = /\brequire(Staff|Admin|Patient|ProofedPatient)\s*\(/;

function dirs(dir: string): string[] {
  return [dir, ...readdirSync(dir).flatMap((n) => (statSync(join(dir, n)).isDirectory() ? dirs(join(dir, n)) : []))];
}

/** Each unguarded part of a `+page.server.ts`: `load`, or `action <name>`. */
export function unguarded(source: string): string[] {
  const out: string[] = [];
  const actionsAt = source.search(/export const actions\b/);
  const loadAt = source.search(/export const load\b/);
  if (loadAt >= 0) {
    const end = actionsAt > loadAt ? actionsAt : source.length;
    if (!GUARD.test(source.slice(loadAt, end))) out.push("load");
  }
  if (actionsAt >= 0) {
    // Actions are the object's top-level properties: `  name: …` at two spaces of indentation.
    const body = source.slice(actionsAt);
    const starts = [...body.matchAll(/^ {2}(\w+)\s*:/gm)];
    starts.forEach((m, i) => {
      const segment = body.slice(m.index, starts[i + 1]?.index ?? body.length);
      if (!GUARD.test(segment)) out.push(`action ${m[1]}`);
    });
  }
  return out;
}

describe("route guards", () => {
  const all = dirs(ROUTES).map((d) => relative(ROUTES, d).split("\\").join("/"));

  it("every page that is not public has a server file whose load and actions are guarded", () => {
    const problems: string[] = [];
    for (const route of all) {
      if (route in PUBLIC) continue;
      const dir = join(ROUTES, route);
      const page = existsSync(join(dir, "+page.svelte"));
      const server = join(dir, "+page.server.ts");
      if (!existsSync(server)) {
        if (page) problems.push(`${route || "/"}: a page with no +page.server.ts, so nothing guards it`);
        continue;
      }
      for (const part of unguarded(readFileSync(server, "utf8"))) problems.push(`${route}: ${part} calls no guard`);
    }
    expect(problems).toEqual([]);
  });

  it("every route listed as public still exists", () => {
    expect(Object.keys(PUBLIC).filter((r) => !all.includes(r))).toEqual([]);
  });

  it("the checker finds an unguarded load and an unguarded action", () => {
    const src = `
export const load = async ({ locals }) => {
  return {};
};
export const actions = {
  save: async ({ locals }) => {
    requireAdmin(locals, "/x");
  },
  remove: async ({ locals }) => {
    return {};
  },
};`;
    expect(unguarded(src)).toEqual(["load", "action remove"]);
    expect(unguarded(`export const load = ({ locals }) => { requireStaff(locals, "/y"); };`)).toEqual([]);
  });
});
