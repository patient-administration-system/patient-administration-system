import type { PageServerLoad } from "./$types";
import { requireStaff } from "#lib/server/guard.js";
import { pasArray } from "#lib/server/pas.js";
import { fromPas, signedInFetch } from "#lib/server/route.js";
import { patientRows, type PersonLike } from "#lib/views.js";

export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/patients");
  const q = (url.searchParams.get("q") ?? "").trim().slice(0, 100);
  if (!q) return { q, rows: [] as Record<string, unknown>[] };
  const people = await fromPas(pasArray<PersonLike>(signedInFetch(fetch, locals), `/api/patients?q=${encodeURIComponent(q)}&limit=50`));
  return { q, rows: patientRows(people) };
};
