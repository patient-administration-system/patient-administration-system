import type { PageServerLoad } from "./$types";
import { oidcConfig } from "#lib/server/config.js";
import { safeReturnPath } from "#lib/server/oidc.js";

const MESSAGES: Record<string, string> = {
  failed: "Sign-in did not complete. Please try again.",
  denied: "Sign-in was cancelled.",
  unavailable: "The sign-in service is not available right now.",
  idle: "You were signed out because you were away for a while.",
};

export const load: PageServerLoad = ({ url }) => ({
  enabled: oidcConfig() !== null,
  message: MESSAGES[url.searchParams.get("error") ?? ""] ?? null,
  returnTo: safeReturnPath(url.searchParams.get("return")),
});
