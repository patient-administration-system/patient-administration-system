import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import { PasError, pas } from "#lib/server/pas.js";
import { patientRows, type PersonLike } from "#lib/views.js";

export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  requireStaff(locals, "/patients");
  const q = (url.searchParams.get("q") ?? "").trim().slice(0, 100);
  if (!q) return { q, rows: [] as Record<string, unknown>[] };
  try {
    const people = await pas<PersonLike[]>(fetch, `/api/patients?q=${encodeURIComponent(q)}&limit=50`);
    return { q, rows: patientRows(Array.isArray(people) ? people : []) };
  } catch (e) {
    if (e instanceof PasError) error(502, e.message);
    throw e;
  }
};
