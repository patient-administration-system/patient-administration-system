import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { navFor } from "#lib/access.js";
import { openSuspicions, transportNotBooked } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

/**
 * WPAS-5 (FR-219): a tile for each area the signed-in role may use. The tiles are the navigation, so a role
 * never sees a tile for an area it cannot open. The suspected-conditions tile says how many are past their
 * target, counted on the first page of the open list (longest wait first, so the past ones come first).
 *
 * PA-01: above the tiles, an alert for each upcoming appointment whose ride nobody booked (no transport
 * provider, a ride the stand-in client made up, or a provider failure), soonest first, from the first page.
 */
export const load: PageServerLoad = async ({ locals, fetch }) => {
  const session = requireStaff(locals, "/home");
  const tiles = navFor(session.user).filter((n) => n.href !== "/home");
  const f = signedInFetch(fetch, locals);
  const [first, rides] = await Promise.all([fromPas(openSuspicions(f, null)), fromPas(transportNotBooked(f))]);
  const past = first.rows.filter((s) => s.target_status === "past").length;
  return {
    tiles,
    past,
    pastIsMore: first.next !== null && past === first.rows.length,
    rides: rides.rows,
    ridesMore: rides.next !== null,
  };
};
