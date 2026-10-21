import type { Actions, PageServerLoad } from "./$types";
import { fail } from "@sveltejs/kit";
import { requireAdmin } from "#lib/server/guard.js";
import { letterTemplates, updateLetterTemplate, type LetterTemplate } from "#lib/server/pas.js";
import { fromPas, pasFailure, signedInFetch } from "#lib/server/route.js";
import { cleanChannels } from "#lib/channels.js";
import { isUuid } from "#lib/access.js";

export const load: PageServerLoad = async ({ locals, fetch }) => {
  requireAdmin(locals, "/admin/reminders");
  return { templates: await fromPas(letterTemplates(signedInFetch(fetch, locals))) };
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

    // Edit only the editable fields of the template as stored, never a browser-supplied copy of the rest. A PAS
    // failure while reading it is the form's error too, not a broken page.
    const f = signedInFetch(fetch, locals);
    let current: LetterTemplate | undefined;
    const unread = await pasFailure(async () => {
      current = (await letterTemplates(f)).find((t) => t.id === id);
    }, { id, messageKey: "admin.error.rejected" });
    if (unread) return unread;
    if (!current) return fail(404, { id, messageKey: "admin.error.notFound" });
    const template = current;
    const failed = await pasFailure(
      () => updateLetterTemplate(f, { ...template, subject, body_tera: body, channels, active: form.get("active") === "on" }),
      { id, messageKey: "admin.error.rejected" },
    );
    return failed ?? { id, saved: true };
  },
};
