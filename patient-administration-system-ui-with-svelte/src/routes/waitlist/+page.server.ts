import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { pasArray, type WaitlistEntry } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";

export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/waitlist");
  const service = (url.searchParams.get("service") ?? "general").trim().slice(0, 80) || "general";
  const entries = await fromPas(pasArray<WaitlistEntry>(signedInFetch(fetch, locals), `/api/waitlist?service=${encodeURIComponent(service)}`));
  return { service, entries };
};
