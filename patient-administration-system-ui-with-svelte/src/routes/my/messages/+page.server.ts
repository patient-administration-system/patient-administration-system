import type { PageServerLoad } from "./$types";
import { requireProofedPatient } from "#lib/server/guard.js";
import { isCursor, portalMessages } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

// Portal messages are the secure channel: SMS and email only say "you have a message"
// and link here, so the detail is shown only after sign-in, and only to its owner. One page at a time,
// newest first; `?older=` carries the cursor to the next (PA-14).
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  const { personId } = await requireProofedPatient(locals, fetch, "/my/messages");
  const older = url.searchParams.get("older");
  const page = await fromPas(portalMessages(signedInFetch(fetch, locals), personId, isCursor(older) ? older : null), {
    notFound: true,
  });
  return { messages: page.rows, olderCursor: page.next };
};
