import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { openSuspicions, SUSPECTED_CONDITIONS, type SuspectedCondition } from "#lib/server/pas.js";
import { pageFromPas, signedInFetch } from "#lib/server/route.js";

/** WPAS-1 (FR-212): open suspected conditions, longest wait first, optionally of one condition. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/suspicions");
  const asked = url.searchParams.get("condition") ?? "";
  const condition = (SUSPECTED_CONDITIONS as readonly string[]).includes(asked) ? (asked as SuspectedCondition) : null;
  const f = signedInFetch(fetch, locals);
  const page = await pageFromPas(url, (cursor) => openSuspicions(f, condition, cursor));
  return {
    condition,
    conditions: SUSPECTED_CONDITIONS,
    rows: page.rows,
    cutAt: page.cutAt,
    nextHref: page.nextHref,
    firstHref: page.firstHref,
  };
};
