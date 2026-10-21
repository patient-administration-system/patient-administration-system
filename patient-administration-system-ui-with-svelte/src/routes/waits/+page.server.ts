import type { Actions, PageServerLoad } from "./$types";
import { fail } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { askStillNeeded, dueChecks } from "#lib/server/pas.js";
import { actorName, fromPas, pasFailure, signedInFetch } from "#lib/server/route.js";
import { isUuid } from "#lib/access.js";

/** CAP-8: long waiters nobody has asked recently whether they still need the appointment. */
export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireStaff(locals, "/waits");
  return { due: await fromPas(dueChecks(signedInFetch(fetch, locals))) };
};

export const actions: Actions = {
  ask: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/waits");
    const pathway = String((await request.formData()).get("pathway") ?? "");
    if (!isUuid(pathway)) return fail(400, { pathway, messageKey: "waits.error" });
    // The person asking is the signed-in member of staff, never a name typed into the form.
    const failed = await pasFailure(() => askStillNeeded(signedInFetch(fetch, locals), pathway, actorName(locals)), {
      pathway,
      messageKey: "waits.error",
    });
    return failed ?? { pathway, asked: true };
  },
};
