import type { PageServerLoad } from "./$types";
import { requireProofedPatient } from "#lib/server/guard.js";
import { portalMessages } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

// Portal messages are the secure channel: SMS and email only say "you have a message"
// and link here, so the detail is shown only after sign-in, and only to its owner.
export const load: PageServerLoad = async ({ locals, fetch }) => {
  const { personId } = await requireProofedPatient(locals, fetch, "/my/messages");
  return { messages: await fromPas(portalMessages(signedInFetch(fetch, locals), personId), { notFound: true }) };
};
