import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import {
  adjustmentsFor,
  appointmentsBetween,
  noshowFor,
  type AppointmentAdjustments,
  type NoShowItem,
} from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

const DAY = 86_400_000;

export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireStaff(locals, "/appointments");
  const now = Date.now();
  const f = signedInFetch(fetch, locals);
  const appointments = await fromPas(
    appointmentsBetween(f, new Date(now - 31 * DAY).toISOString(), new Date(now + 62 * DAY).toISOString()),
  );
  // The no-show flag is support for the front desk (CAP-12). It is only ever an addition to the page: while the
  // score is off, or if asking fails, the calendar is exactly what it was.
  let noshow: NoShowItem[] = [];
  // Reasonable adjustments (NEW-6) are the same kind of addition: what the patient needs, beside the appointment.
  let adjustments: AppointmentAdjustments[] = [];
  const ids = appointments.filter((a) => a.status === "booked" && Date.parse(a.start_datetime) > now).map((a) => a.id);
  try {
    if (ids.length > 0) noshow = (await noshowFor(f, ids.slice(0, 500))).items;
  } catch {
    noshow = [];
  }
  try {
    if (ids.length > 0) adjustments = await adjustmentsFor(f, ids.slice(0, 500));
  } catch {
    adjustments = [];
  }
  return { appointments, noshow, adjustments };
};
