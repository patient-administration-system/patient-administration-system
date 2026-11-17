// The language is whatever the URL says, derived here (not in a server load) so it is recomputed on
// every navigation, including client-side ones within the same route. A server layout load that
// does not read the URL is never re-run, and the page would stay in the old language.
import type { LayoutLoad } from "./$types";
import { DEFAULT_LOCALE, localeOfPath } from "#lib/locales.js";
import { ensureLocale } from "#lib/i18n.svelte.js";

// `data` is the server layout load's result (user name, navigation); a universal load must pass it on.
// The browser holds English only until the page's language is loaded here, before the page renders.
export const load: LayoutLoad = async ({ url, data }) => {
  const locale = localeOfPath(url.pathname) ?? DEFAULT_LOCALE;
  await ensureLocale(locale);
  return { ...data, locale };
};
