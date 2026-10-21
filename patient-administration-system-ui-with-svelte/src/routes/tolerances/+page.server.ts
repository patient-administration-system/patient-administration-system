import type { Actions, PageServerLoad } from "./$types";
import { fail } from "@sveltejs/kit";
import { requireAdmin, requireStaff } from "#lib/server/guard.js";
import { acknowledgeToleranceAlert, toleranceAlerts, toleranceStatus } from "#lib/server/pas.js";
import { actorName, fromPas, pasFailure, signedInFetch } from "#lib/server/route.js";
import { isAdmin, isUuid } from "#lib/access.js";

const WINDOWS = [7, 30, 90];

/** Time, money and resources against their tolerances, in red, amber and green, and the open alerts. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  const session = requireStaff(locals, "/tolerances");
  const asked = Number(url.searchParams.get("days"));
  const days = WINDOWS.includes(asked) ? asked : 30;
  const f = signedInFetch(fetch, locals);
  return { days, windows: WINDOWS, canAcknowledge: isAdmin(session.user), status: await fromPas(toleranceStatus(f, days)), alerts: await fromPas(toleranceAlerts(f)) };
};

export const actions: Actions = {
  acknowledge: async ({ request, locals, fetch }) => {
    // The project administrator owns the alerts (TOL-1).
    requireAdmin(locals, "/tolerances");
    const id = String((await request.formData()).get("alert") ?? "");
    if (!isUuid(id)) return fail(400, { id, messageKey: "tol.alerts.error" });
    // The person acknowledging is the signed-in member of staff, never a name typed into the form.
    const failed = await pasFailure(() => acknowledgeToleranceAlert(signedInFetch(fetch, locals), id, actorName(locals)), {
      id,
      messageKey: "tol.alerts.error",
    });
    return failed ?? { id, acknowledged: true };
  },
};
