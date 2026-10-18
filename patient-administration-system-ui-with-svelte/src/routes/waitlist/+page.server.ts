import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { PasError, pas, type WaitlistEntry } from "#lib/server/pas.js";

export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/waitlist");
  const service = (url.searchParams.get("service") ?? "general").trim().slice(0, 80) || "general";
  try {
    return { service, entries: await pas<WaitlistEntry[]>(fetch, `/api/waitlist?service=${encodeURIComponent(service)}`) };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};
