import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requirePatient } from "#lib/server/guard.js";
import { PasError, patientAppointments } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";

// A patient sees only their own appointments: the person id comes from the
// verified ID token, never from the URL or the browser.
export const load: PageServerLoad = async ({ locals, fetch }) => {
  const { personId } = requirePatient(locals, "/my/appointments");
  try {
    return { appointments: await patientAppointments(actingAs(fetch, locals.oidc?.user.sub), personId) };
  } catch (e) {
    if (e instanceof PasError) error(e.status === 404 ? 404 : 502, e.message);
    throw e;
  }
};
