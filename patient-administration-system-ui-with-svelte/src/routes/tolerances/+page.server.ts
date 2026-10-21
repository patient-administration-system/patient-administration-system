import type { Actions, PageServerLoad } from "./$types";
import { error, fail } from "@sveltejs/kit";
import { requireAdmin, requireStaff } from "#lib/server/guard.js";
import { PasError, acknowledgeToleranceAlert, toleranceAlerts, toleranceStatus } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";
import { isAdmin, isUuid } from "#lib/access.js";

const WINDOWS = [7, 30, 90];

/** Time, money and resources against their tolerances, in red, amber and green, and the open alerts. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  const session = requireStaff(locals, "/tolerances");
  const asked = Number(url.searchParams.get("days"));
  const days = WINDOWS.includes(asked) ? asked : 30;
  const f = actingAs(fetch, locals.oidc?.user.sub);
  try {
    return { days, windows: WINDOWS, canAcknowledge: isAdmin(session.user), status: await toleranceStatus(f, days), alerts: await toleranceAlerts(f) };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};

export const actions: Actions = {
  acknowledge: async ({ request, locals, fetch }) => {
    // The project administrator owns the alerts (TOL-1).
    requireAdmin(locals, "/tolerances");
    const id = String((await request.formData()).get("alert") ?? "");
    if (!isUuid(id)) return fail(400, { id, messageKey: "tol.alerts.error" });
    // The person acknowledging is the signed-in member of staff, never a name typed into the form.
    const user = locals.oidc?.user;
    const by = (user?.name ?? user?.sub ?? "").toString().slice(0, 120);
    try {
      await acknowledgeToleranceAlert(actingAs(fetch, user?.sub), id, by);
    } catch (e) {
      if (e instanceof PasError) return fail(e.status >= 500 ? 502 : 400, { id, messageKey: "tol.alerts.error" });
      throw e;
    }
    return { id, acknowledged: true };
  },
};
