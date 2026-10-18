import type { Actions, PageServerLoad } from "./$types";
import { error, fail } from "@sveltejs/kit";
import { requireAdmin } from "#lib/server/guard.js";
import { PasError, letterTemplates, updateLetterTemplate } from "#lib/server/pas.js";
import { actingAs } from "#lib/acting-as.js";
import { cleanChannels } from "#lib/channels.js";
import { isUuid } from "#lib/access.js";

export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireAdmin(locals, "/admin/reminders");
  try {
    return { templates: await letterTemplates(actingAs(fetch, locals.oidc?.user.sub)) };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};

export const actions: Actions = {
  save: async ({ request, locals, fetch }) => {
    requireAdmin(locals, "/admin/reminders");
    const form = await request.formData();
    const id = String(form.get("id") ?? "");
    if (!isUuid(id)) return fail(400, { id, messageKey: "admin.error.unknownTemplate" });
    const subject = String(form.get("subject") ?? "").trim();
    const body = String(form.get("body_tera") ?? "").trim();
    if (!subject || !body) return fail(400, { id, messageKey: "admin.error.required" });
    const channels = cleanChannels(form.getAll("channels").map(String));
    if (channels.length === 0) return fail(400, { id, messageKey: "admin.error.channelRequired" });

    // Edit only the editable fields of the template as stored, never a browser-supplied copy of the rest.
    const current = (await letterTemplates(actingAs(fetch, locals.oidc?.user.sub))).find((t) => t.id === id);
    if (!current) return fail(404, { id, messageKey: "admin.error.notFound" });
    try {
      await updateLetterTemplate(actingAs(fetch, locals.oidc?.user.sub), {
        ...current,
        subject,
        body_tera: body,
        channels,
        active: form.get("active") === "on",
      });
    } catch (e) {
      if (e instanceof PasError) return fail(e.status >= 500 ? 502 : 400, { id, messageKey: "admin.error.rejected" });
      throw e;
    }
    return { id, saved: true };
  },
};
