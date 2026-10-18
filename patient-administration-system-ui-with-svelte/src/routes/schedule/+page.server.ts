import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { PasError, appointmentsBetween } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";

const DAY = 86_400_000;

export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireStaff(locals, "/schedule");
  const start = new Date();
  start.setUTCHours(0, 0, 0, 0);
  try {
    return {
      today: start.toISOString().slice(0, 10),
      appointments: await appointmentsBetween(actingAs(fetch, locals.oidc?.user.sub), start.toISOString(), new Date(start.getTime() + 28 * DAY).toISOString()),
    };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};
