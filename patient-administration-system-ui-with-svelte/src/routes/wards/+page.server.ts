import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { isUuid } from "#lib/access.js";
import { facilitiesWithWards, wardOverview } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

/** WPAS-4 (FR-218): every bed of a ward and who is in it. Nothing clinical. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/wards");
  const ward = (url.searchParams.get("ward") ?? "").trim();
  const f = signedInFetch(fetch, locals);
  // With no ward chosen, list the wards to choose from.
  if (!ward) return { ward, invalid: false, overview: null, places: await fromPas(facilitiesWithWards(f)) };
  if (!isUuid(ward)) return { ward, invalid: true, overview: null, places: [] };
  return { ward, invalid: false, overview: await fromPas(wardOverview(f, ward)), places: [] };
};
