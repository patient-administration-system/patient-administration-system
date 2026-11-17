import type { Cookies } from "@sveltejs/kit";
import type { Handle, HandleFetch } from "@sveltejs/kit/hooks";
import { redirect } from "@sveltejs/kit";

import { OIDC_COOKIE, OIDC_COOKIE_OPTIONS } from "#lib/server/oidc-session.js";
import { sessions } from "#lib/server/session-store-instance.js";
import { PAS_API_URL, oidcConfig, oidcOptions } from "#lib/server/config.js";
import { REQUEST_ID, requestIdFor, withRequestId } from "#lib/server/request-id.js";
import { allowFormActionTo, applySecurityHeaders, isHttps } from "#lib/security-headers.js";
import {
  DEFAULT_LOCALE,
  type Locale,
  isRtl,
  isUnlocalized,
  localeFromAcceptLanguage,
  localizePath,
  normalizeLocale,
  splitPath,
} from "#lib/locales.js";
import { THEME_COOKIE, dataThemeFor, themeFrom } from "#lib/theme.js";
import { ensureLocale } from "#lib/i18n.svelte.js";

/** Remembers the reader's locale, to choose one for a bare, unprefixed URL. */
export const LOCALE_COOKIE = "locale";

/** The locale for a request that names none: the remembered one, else `Accept-Language`, else English. */
export function negotiateLocale(cookies: Pick<Cookies, "get">, acceptLanguage: string | null): Locale {
  return normalizeLocale(cookies.get(LOCALE_COOKIE)) ?? localeFromAcceptLanguage(acceptLanguage) ?? DEFAULT_LOCALE;
}

sessions().setIdleSeconds(oidcOptions().idleSeconds);

/** The identity provider's origin when sign-out also ends the provider's session, else null. */
function providerSignoutOrigin(): string | null {
  const issuer = oidcOptions().endProviderSession ? oidcConfig()?.issuer : undefined;
  try {
    return issuer ? new URL(issuer).origin : null;
  } catch {
    return null;
  }
}
const SIGNOUT_ORIGIN = providerSignoutOrigin();

// BFF entry point: resolve the httpOnly session cookie to a server-side OIDC session, and the URL
// to a locale, on every request. The browser never holds a token.
export const handle: Handle = async ({ event, resolve }) => {
  event.locals.requestId = requestIdFor(event.request.headers.get(REQUEST_ID));
  const negotiated = negotiateLocale(event.cookies, event.request.headers.get("accept-language"));
  const cookie = event.cookies.get(OIDC_COOKIE);
  event.locals.oidc = await sessions().get(cookie);
  if (!event.locals.oidc && (await sessions().endedIdle(cookie))) {
    event.cookies.delete(OIDC_COOKIE, OIDC_COOKIE_OPTIONS);
    const { pathname } = event.url;
    if (event.request.headers.get("sec-fetch-dest") === "document" && !pathname.includes("/signin")) {
      redirect(303, `${localizePath("/signin", negotiated)}?error=idle`);
    }
  }

  const { pathname, search } = event.url;
  if (!isUnlocalized(pathname)) {
    const { locale, rest } = splitPath(pathname);
    const reading = event.request.method === "GET" || event.request.method === "HEAD";
    if (!locale && reading) {
      // A bare URL (a bookmark, a link in a text message) gains its locale prefix.
      redirect(302, `${localizePath(rest, negotiated)}${search}`);
    }
    if (locale) {
      event.locals.locale = locale;
      if (event.cookies.get(LOCALE_COOKIE) !== locale) {
        event.cookies.set(LOCALE_COOKIE, locale, {
          path: "/",
          maxAge: 60 * 60 * 24 * 365,
          sameSite: "lax",
          httpOnly: false,
          secure: event.url.protocol === "https:",
        });
      }
    }
  }
  const locale = event.locals.locale ?? negotiated;
  event.locals.locale = locale;
  // Only English is loaded at the start; this request's language is loaded before any load function or page
  // can read it (a server load runs before the layout's universal load).
  await ensureLocale(locale);
  event.locals.theme = themeFrom(event.cookies.get(THEME_COOKIE));

  const response = await resolve(event, {
    // The static shell carries the right `lang`/`dir`, and the person's light or dark choice,
    // before any script runs (NFR-30: no flash of the other theme).
    transformPageChunk: ({ html }) =>
      html
        .replace('lang="%lang%"', `lang="${locale}"`)
        .replace('dir="%dir%"', `dir="${isRtl(locale) ? "rtl" : "ltr"}"`)
        .replace('data-theme="%theme%"', `data-theme="${dataThemeFor(event.locals.theme)}"`),
  });
  try {
    applySecurityHeaders(response.headers, isHttps(event.url, event.request.headers.get("x-forwarded-proto")));
    response.headers.set(REQUEST_ID, event.locals.requestId);
    const csp = response.headers.get("content-security-policy");
    if (csp && SIGNOUT_ORIGIN) response.headers.set("content-security-policy", allowFormActionTo(csp, SIGNOUT_ORIGIN));
  } catch {
    // Immutable headers (e.g. a redirect response from `fetch`): leave as is.
  }
  return response;
};

// Every server-side call to the PAS API carries this request's id, so the API's log lines for it can be found
// from the page's (AXA-6 did the same for the operator front-end).
export const handleFetch: HandleFetch = async ({ event, request, fetch }) =>
  fetch(withRequestId(request, event.locals.requestId, PAS_API_URL));
