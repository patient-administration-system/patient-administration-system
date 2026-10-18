// Finish the sign-in. Every check fails closed to the sign-in page with a
// generic error (the reason is logged by code, never with a token): state must
// match both the cookie and a pending server-side login (single use), the code
// must exchange with the PKCE verifier, and the ID token must verify.
import type { RequestHandler } from "./$types";
import { error, redirect } from "@sveltejs/kit";
import { oidcConfig, oidcOptions } from "#lib/server/config.js";
import { OidcError, exchangeCode, verifyIdToken } from "#lib/server/oidc.js";
import {
  OIDC_COOKIE,
  OIDC_COOKIE_OPTIONS,
  OIDC_TX_COOKIE,
  providerMetadata,
  signingKeys,
} from "#lib/server/oidc-session.js";
import { sessions } from "#lib/server/session-store-instance.js";
import { DEFAULT_LOCALE, localizePath, normalizeLocale } from "#lib/locales.js";

export const GET: RequestHandler = async ({ url, cookies, fetch, locals }) => {
  const cfg = oidcConfig();
  if (!cfg) error(404, "Sign-in is not configured");
  // The callback is not locale-prefixed (the provider needs one fixed address), so the reader's
  // language comes from the login that was started in it.
  let language = locals.locale ?? DEFAULT_LOCALE;
  const signin = (error: string) => `${localizePath("/signin", language)}?error=${error}`;
  const fail = (reason: string): never => {
    console.warn(`oidc sign-in refused: ${reason}`);
    redirect(303, signin("failed"));
  };

  const state = url.searchParams.get("state") ?? "";
  const bound = cookies.get(OIDC_TX_COOKIE) ?? "";
  cookies.delete(OIDC_TX_COOKIE, { path: "/" });
  const tx = state ? await sessions().takeTransaction(state) : null;
  if (tx) language = normalizeLocale(tx.locale) ?? language;
  if (!tx || !bound || bound !== state) fail("state");

  const denied = url.searchParams.get("error");
  if (denied) {
    console.warn(`oidc sign-in refused: provider error ${denied}`);
    redirect(303, signin(denied === "access_denied" ? "denied" : "failed"));
  }
  const code = url.searchParams.get("code");
  if (!code) fail("no code");

  let session;
  try {
    const meta = await providerMetadata(fetch, cfg.issuer);
    const tokens = await exchangeCode(fetch, meta, cfg, {
      code: code as string,
      verifier: tx!.verifier,
      redirectUri: tx!.redirectUri,
    });
    if (!tokens.id_token) throw new OidcError("id_token", "no ID token");
    const expect = { issuer: meta.issuer, audience: cfg.clientId, nonce: tx!.nonce, tenantId: cfg.tenantId };
    let claims;
    try {
      claims = verifyIdToken(tokens.id_token, await signingKeys(fetch, meta), expect);
    } catch (e) {
      // A key may have rotated since the key set was cached: re-read once.
      if (!(e instanceof OidcError) || e.message !== "no matching signing key") throw e;
      claims = verifyIdToken(tokens.id_token, await signingKeys(fetch, meta, true), expect);
    }
    session = await sessions().createSession({
      accessToken: tokens.access_token,
      refreshToken: tokens.refresh_token,
      idToken: tokens.id_token,
      expiresInSeconds: tokens.expires_in,
      sessionSeconds: oidcOptions().sessionSeconds,
      claims,
    });
  } catch (e) {
    fail(e instanceof OidcError ? e.code : "unexpected");
  }

  cookies.set(OIDC_COOKIE, session!.id, { ...OIDC_COOKIE_OPTIONS, maxAge: oidcOptions().sessionSeconds });
  redirect(303, tx!.returnTo);
};
