import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { appointmentsBetween } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";
import { clinicTimeZone } from "#lib/server/config.js";
import { startOfDayIn, todayIn } from "#lib/views.js";

const DAY = 86_400_000;

export const load: PageServerLoad = async ({ locals, fetch }) => {
  const zone = clinicTimeZone();
  requireStaff(locals, "/schedule");
  // Four weeks from the clinic's midnight today, so an appointment just after midnight on the clinic's clock is in.
  const now = Date.now();
  const start = startOfDayIn(zone, now);
  const end = new Date(start.getTime() + 28 * DAY);
  return {
    today: todayIn(zone, now),
    timeZone: zone,
    appointments: await fromPas(
      appointmentsBetween(signedInFetch(fetch, locals), start.toISOString(), end.toISOString()),
    ),
  };
};
