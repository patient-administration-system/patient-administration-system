import type { Actions, PageServerLoad } from "./$types";
import { error, fail } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { PasError, askStillNeeded, dueChecks } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";
import { isUuid } from "#lib/access.js";

/** CAP-8: long waiters nobody has asked recently whether they still need the appointment. */
export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireStaff(locals, "/waits");
  try {
    return { due: await dueChecks(actingAs(fetch, locals.oidc?.user.sub)) };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};

export const actions: Actions = {
  ask: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/waits");
    const pathway = String((await request.formData()).get("pathway") ?? "");
    if (!isUuid(pathway)) return fail(400, { pathway, messageKey: "waits.error" });
    // The person asking is the signed-in member of staff, never a name typed into the form.
    const user = locals.oidc?.user;
    const by = (user?.name ?? user?.sub ?? "").toString().slice(0, 120);
    try {
      await askStillNeeded(actingAs(fetch, user?.sub), pathway, by);
    } catch (e) {
      if (e instanceof PasError) return fail(e.status >= 500 ? 502 : 400, { pathway, messageKey: "waits.error" });
      throw e;
    }
    return { pathway, asked: true };
  },
};
