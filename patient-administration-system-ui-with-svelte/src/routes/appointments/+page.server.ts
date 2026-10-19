import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { PasError, appointmentsBetween, noshowFor, type NoShowItem } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";

const DAY = 86_400_000;

export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireStaff(locals, "/appointments");
  const now = Date.now();
  try {
    const f = actingAs(fetch, locals.oidc?.user.sub);
    const appointments = await appointmentsBetween(f, new Date(now - 31 * DAY).toISOString(), new Date(now + 62 * DAY).toISOString());
    // The no-show flag is support for the front desk (CAP-12). It is only ever an addition to the page: while the
    // score is off, or if asking fails, the calendar is exactly what it was.
    let noshow: NoShowItem[] = [];
    try {
      const ids = appointments.filter((a) => a.status === "booked" && Date.parse(a.start_datetime) > now).map((a) => a.id);
      if (ids.length > 0) noshow = (await noshowFor(f, ids.slice(0, 500))).items;
    } catch {
      noshow = [];
    }
    return { appointments, noshow };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};
