import type { PageServerLoad } from "./$types";
import { requireProofedPatient } from "#lib/server/guard.js";
import { isCursor, patientAppointments } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";
import { clinicTimeZone } from "#lib/server/config.js";
import { todayIn } from "#lib/views.js";

// A patient sees only their own appointments: the person id comes from the
// verified ID token, never from the URL or the browser. The visits to come are read on their own and the
// history a page at a time (`?older=` carries the next page's cursor), so a long record stays bounded (PA-14).
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  const zone = clinicTimeZone();
  const { personId } = await requireProofedPatient(locals, fetch, "/my/appointments");
  const f = signedInFetch(fetch, locals);
  const older = url.searchParams.get("older");
  const [upcoming, past] = await Promise.all([
    fromPas(patientAppointments(f, personId, "upcoming"), { notFound: true }),
    fromPas(patientAppointments(f, personId, "past", isCursor(older) ? older : null), { notFound: true }),
  ]);
  return {
    upcoming: upcoming.rows,
    past: past.rows,
    olderCursor: past.next,
    today: todayIn(zone, Date.now()),
    timeZone: zone,
  };
};
