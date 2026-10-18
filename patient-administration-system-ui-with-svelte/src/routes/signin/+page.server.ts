import type { PageServerLoad } from "./$types";
import { oidcConfig } from "#lib/server/config.js";
import { safeReturnPath } from "#lib/server/oidc.js";

// Error codes are catalogue keys (`signin.error.<code>`), translated where they are shown.
const ERRORS = new Set(["failed", "denied", "unavailable", "idle"]);

export const load: PageServerLoad = ({ url }) => {
  const code = url.searchParams.get("error") ?? "";
  return {
    enabled: oidcConfig() !== null,
    errorKey: ERRORS.has(code) ? `signin.error.${code}` : null,
    returnTo: safeReturnPath(url.searchParams.get("return")),
  };
};
