import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { PasError, appointmentsBetween } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";

const DAY = 86_400_000;

export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireStaff(locals, "/appointments");
  const now = Date.now();
  try {
    return {
      appointments: await appointmentsBetween(
        actingAs(fetch, locals.oidc?.user.sub),
        new Date(now - 31 * DAY).toISOString(),
        new Date(now + 62 * DAY).toISOString(),
      ),
    };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};
