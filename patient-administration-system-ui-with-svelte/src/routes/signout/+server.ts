import type { RequestHandler } from "./$types";
import { redirect } from "@sveltejs/kit";
import { oidcConfig, oidcOptions } from "#lib/server/config.js";
import { OIDC_COOKIE, OIDC_COOKIE_OPTIONS, providerMetadata } from "#lib/server/oidc-session.js";
import { sessions } from "#lib/server/session-store-instance.js";

// POST only: a link (GET) must never sign anyone out.
export const POST: RequestHandler = async ({ cookies, locals, url, fetch }) => {
  const idToken = locals.oidc?.idToken;
  await sessions().delete(cookies.get(OIDC_COOKIE));
  cookies.delete(OIDC_COOKIE, OIDC_COOKIE_OPTIONS);
  const cfg = oidcConfig();
  if (cfg && idToken && oidcOptions().endProviderSession) {
    try {
      const meta = await providerMetadata(fetch, cfg.issuer);
      if (meta.end_session_endpoint) {
        const q = new URLSearchParams({ id_token_hint: idToken, post_logout_redirect_uri: `${url.origin}/signin` });
        redirect(303, `${meta.end_session_endpoint}?${q}`, { external: true });
      }
    } catch (e) {
      if (e && typeof e === "object" && "status" in e) throw e;
    }
  }
  redirect(303, "/signin");
};
