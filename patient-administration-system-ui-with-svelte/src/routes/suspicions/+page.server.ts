import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { openSuspicions, suspicionProfile } from "#lib/server/pas.js";
import { fromPas, pageFromPas, signedInFetch } from "#lib/server/route.js";

/** WPAS-1, WPAS-8 (FR-212): open suspected conditions, longest wait first, optionally of one condition. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/suspicions");
  const f = signedInFetch(fetch, locals);
  const profile = await fromPas(suspicionProfile(f));
  const conditions = profile.conditions.map((c) => ({ code: c.code, name: c.name }));
  const asked = url.searchParams.get("condition") ?? "";
  const condition = conditions.some((c) => c.code === asked) ? asked : null;
  const page = await pageFromPas(url, (cursor) => openSuspicions(f, condition, cursor));
  return {
    condition,
    conditions,
    rows: page.rows,
    cutAt: page.cutAt,
    nextHref: page.nextHref,
    firstHref: page.firstHref,
  };
};
