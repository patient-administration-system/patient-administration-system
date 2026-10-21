import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { appointmentsBetween } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

const DAY = 86_400_000;

export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireStaff(locals, "/schedule");
  const start = new Date();
  start.setUTCHours(0, 0, 0, 0);
  const end = new Date(start.getTime() + 28 * DAY);
  return {
    today: start.toISOString().slice(0, 10),
    appointments: await fromPas(
      appointmentsBetween(signedInFetch(fetch, locals), start.toISOString(), end.toISOString()),
    ),
  };
};
