import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { PasError, appointmentHistory, contactLog, type HistoryRow } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";
import { isUuid } from "#lib/access.js";

/** CAP-6: what happened to one appointment, or what was sent to and received from one patient. Read only. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/history");
  const appointment = (url.searchParams.get("appointment") ?? "").trim();
  const patient = (url.searchParams.get("patient") ?? "").trim();
  const asked = appointment || patient;
  if (!asked) return { appointment, patient, rows: null as HistoryRow[] | null, kind: null as "appointment" | "patient" | null, invalid: false };
  if (!isUuid(asked)) return { appointment, patient, rows: null, kind: null, invalid: true };
  const f = actingAs(fetch, locals.oidc?.user.sub);
  try {
    return appointment
      ? { appointment, patient, rows: await appointmentHistory(f, appointment), kind: "appointment" as const, invalid: false }
      : { appointment, patient, rows: await contactLog(f, patient), kind: "patient" as const, invalid: false };
  } catch (e) {
    if (e instanceof PasError) error(e.status === 404 ? 404 : 502, e.message);
    throw e;
  }
};
