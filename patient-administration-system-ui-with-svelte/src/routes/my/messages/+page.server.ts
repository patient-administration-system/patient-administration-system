import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requirePatient } from "#lib/server/guard.js";
import { PasError, portalMessages } from "#lib/server/pas.js";

// Portal messages are the secure channel: SMS and email only say "you have a message"
// and link here, so the detail is shown only after sign-in, and only to its owner.
export const load: PageServerLoad = async ({ locals, fetch }) => {
  const { personId } = requirePatient(locals, "/my/messages");
  try {
    return { messages: await portalMessages(fetch, personId) };
  } catch (e) {
    if (e instanceof PasError) error(e.status === 404 ? 404 : 502, e.message);
    throw e;
  }
};
