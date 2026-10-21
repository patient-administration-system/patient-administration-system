import type { PageServerLoad } from "./$types";
import { requireProofedPatient } from "#lib/server/guard.js";
import { patientAppointments } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

// A patient sees only their own appointments: the person id comes from the
// verified ID token, never from the URL or the browser.
export const load: PageServerLoad = async ({ locals, fetch }) => {
  const { personId } = await requireProofedPatient(locals, fetch, "/my/appointments");
  return { appointments: await fromPas(patientAppointments(signedInFetch(fetch, locals), personId), { notFound: true }) };
};
