import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { isUuid } from "#lib/access.js";
import { patientEnquiry } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

/** WPAS-3 (FR-217): everything the PAS holds for one patient. The PAS records the read. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/enquiry");
  const patient = (url.searchParams.get("patient") ?? "").trim();
  if (!patient) return { patient, invalid: false, enquiry: null };
  if (!isUuid(patient)) return { patient, invalid: true, enquiry: null };
  return { patient, invalid: false, enquiry: await fromPas(patientEnquiry(signedInFetch(fetch, locals), patient)) };
};
