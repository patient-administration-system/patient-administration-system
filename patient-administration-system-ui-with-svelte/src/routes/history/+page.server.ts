import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { appointmentHistory, contactLog, type HistoryRow } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";
import { isUuid } from "#lib/access.js";

/** CAP-6: what happened to one appointment, or what was sent to and received from one patient. Read only. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/history");
  const appointment = (url.searchParams.get("appointment") ?? "").trim();
  const patient = (url.searchParams.get("patient") ?? "").trim();
  const asked = appointment || patient;
  if (!asked) return { appointment, patient, rows: null as HistoryRow[] | null, kind: null as "appointment" | "patient" | null, invalid: false };
  if (!isUuid(asked)) return { appointment, patient, rows: null, kind: null, invalid: true };
  const f = signedInFetch(fetch, locals);
  return appointment
    ? { appointment, patient, rows: await fromPas(appointmentHistory(f, appointment), { notFound: true }), kind: "appointment" as const, invalid: false }
    : { appointment, patient, rows: await fromPas(contactLog(f, patient), { notFound: true }), kind: "patient" as const, invalid: false };
};
