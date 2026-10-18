import type { PageServerLoad } from "./$types";
import { redirect } from "@sveltejs/kit";
import { navFor } from "#lib/access.js";

// Land each person on the first page their role may use.
export const load: PageServerLoad = ({ locals }) => {
  const first = navFor(locals.oidc?.user)[0];
  redirect(303, first ? first.href : "/signin");
};
