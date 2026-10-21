import type { Cookies } from "@sveltejs/kit";
import type { Handle } from "@sveltejs/kit/hooks";
import { redirect } from "@sveltejs/kit";

import { OIDC_COOKIE, OIDC_COOKIE_OPTIONS } from "#lib/server/oidc-session.js";
import { sessions } from "#lib/server/session-store-instance.js";
import { oidcOptions } from "#lib/server/config.js";
import { applySecurityHeaders, isHttps } from "#lib/security-headers.js";
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

/** Remembers the reader's locale, to choose one for a bare, unprefixed URL. */
export const LOCALE_COOKIE = "locale";

/** The locale for a request that names none: the remembered one, else `Accept-Language`, else English. */
export function negotiateLocale(cookies: Pick<Cookies, "get">, acceptLanguage: string | null): Locale {
  return normalizeLocale(cookies.get(LOCALE_COOKIE)) ?? localeFromAcceptLanguage(acceptLanguage) ?? DEFAULT_LOCALE;
}

sessions().setIdleSeconds(oidcOptions().idleSeconds);

// BFF entry point: resolve the httpOnly session cookie to a server-side OIDC session, and the URL
// to a locale, on every request. The browser never holds a token.
export const handle: Handle = async ({ event, resolve }) => {
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

  const response = await resolve(event, {
    // The static shell carries the right `lang`/`dir` before any script runs.
    transformPageChunk: ({ html }) =>
      html
        .replace('lang="%lang%"', `lang="${locale}"`)
        .replace('dir="%dir%"', `dir="${isRtl(locale) ? "rtl" : "ltr"}"`),
  });
  try {
    applySecurityHeaders(response.headers, isHttps(event.url, event.request.headers.get("x-forwarded-proto")));
  } catch {
    // Immutable headers (e.g. a redirect response from `fetch`): leave as is.
  }
  return response;
};
