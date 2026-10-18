import type { Handle } from "@sveltejs/kit/hooks";
import { redirect } from "@sveltejs/kit";

import { OIDC_COOKIE, OIDC_COOKIE_OPTIONS } from "#lib/server/oidc-session.js";
import { sessions } from "#lib/server/session-store-instance.js";
import { oidcOptions } from "#lib/server/config.js";
import { applySecurityHeaders, isHttps } from "#lib/security-headers.js";

sessions().setIdleSeconds(oidcOptions().idleSeconds);

// BFF entry point: resolve the httpOnly session cookie to a server-side OIDC
// session on every request. The browser never holds a token.
export const handle: Handle = async ({ event, resolve }) => {
  const cookie = event.cookies.get(OIDC_COOKIE);
  event.locals.oidc = await sessions().get(cookie);
  if (!event.locals.oidc && (await sessions().endedIdle(cookie))) {
    event.cookies.delete(OIDC_COOKIE, OIDC_COOKIE_OPTIONS);
    const { pathname } = event.url;
    if (
      event.request.headers.get("sec-fetch-dest") === "document" &&
      !pathname.startsWith("/signin")
    ) {
      redirect(303, "/signin?error=idle");
    }
  }
  const response = await resolve(event);
  try {
    applySecurityHeaders(
      response.headers,
      isHttps(event.url, event.request.headers.get("x-forwarded-proto")),
    );
  } catch {
    // Immutable headers (e.g. a redirect response from `fetch`): leave as is.
  }
  return response;
};
