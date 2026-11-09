import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { navFor } from "#lib/access.js";
import { openSuspicions } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

/**
 * WPAS-5 (FR-219): a tile for each area the signed-in role may use. The tiles are the navigation, so a role
 * never sees a tile for an area it cannot open. The suspected-conditions tile says how many are past their
 * target, counted on the first page of the open list (longest wait first, so the past ones come first).
 */
export const load: PageServerLoad = async ({ locals, fetch }) => {
  const session = requireStaff(locals, "/home");
  const tiles = navFor(session.user).filter((n) => n.href !== "/home");
  const first = await fromPas(openSuspicions(signedInFetch(fetch, locals), null));
  const past = first.rows.filter((s) => s.target_status === "past").length;
  return { tiles, past, pastIsMore: first.next !== null && past === first.rows.length };
};
