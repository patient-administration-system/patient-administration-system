import type { LayoutServerLoad } from "./$types";
import { navFor } from "#lib/access.js";

export const load: LayoutServerLoad = ({ locals }) => {
  const user = locals.oidc?.user ?? null;
  return { userName: user?.name ?? null, nav: navFor(user) };
};
