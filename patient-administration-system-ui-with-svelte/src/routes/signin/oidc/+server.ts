// Start the OpenID Connect sign-in (authorization code + PKCE) against Keycloak.
// A plain navigation (an <a href>), so `form-action 'self'` is not in the way.
import type { RequestHandler } from "./$types";
import { error, redirect } from "@sveltejs/kit";
import { oidcConfig, oidcOptions } from "#lib/server/config.js";
import { authorizationUrl, pkcePair, randomToken, safeReturnPath } from "#lib/server/oidc.js";
import { localizePath } from "#lib/locales.js";
import { OIDC_COOKIE_OPTIONS, OIDC_TX_COOKIE, TX_TTL_MS, providerMetadata } from "#lib/server/oidc-session.js";
import { sessions } from "#lib/server/session-store-instance.js";

export const GET: RequestHandler = async ({ url, cookies, fetch, locals }) => {
  const cfg = oidcConfig();
  if (!cfg) error(404, "Sign-in is not configured");
  let meta;
  try {
    meta = await providerMetadata(fetch, cfg.issuer);
  } catch {
    redirect(303, `${localizePath("/signin", locals.locale)}?error=unavailable`);
  }
  const state = randomToken();
  const nonce = randomToken();
  const { verifier, challenge } = pkcePair();
  const redirectUri = oidcOptions().redirectUri ?? `${url.origin}/signin/oidc/callback`;
  await sessions().startTransaction(state, {
    nonce,
    verifier,
    // Remember where to land, in the reader's own language.
    returnTo: localizePath(safeReturnPath(url.searchParams.get("return")), locals.locale),
    redirectUri,
    locale: locals.locale,
  });
  cookies.set(OIDC_TX_COOKIE, state, { ...OIDC_COOKIE_OPTIONS, maxAge: TX_TTL_MS / 1000 });
  redirect(303, authorizationUrl(meta, cfg, { state, nonce, challenge, redirectUri }), { external: true });
};
