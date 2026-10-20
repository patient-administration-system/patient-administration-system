import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { PasError, toleranceStatus } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";

const WINDOWS = [7, 30, 90];

/** Time, money and resources against their tolerances, in red, amber and green. Read only. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/tolerances");
  const asked = Number(url.searchParams.get("days"));
  const days = WINDOWS.includes(asked) ? asked : 30;
  try {
    return { days, windows: WINDOWS, status: await toleranceStatus(actingAs(fetch, locals.oidc?.user.sub), days) };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};
