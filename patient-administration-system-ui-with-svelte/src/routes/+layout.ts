// The language is whatever the URL says, derived here (not in a server load) so it is recomputed on
// every navigation, including client-side ones within the same route. A server layout load that
// does not read the URL is never re-run, and the page would stay in the old language.
import type { LayoutLoad } from "./$types";
import { DEFAULT_LOCALE, localeOfPath } from "#lib/locales.js";

// `data` is the server layout load's result (user name, navigation); a universal load must pass it on.
export const load: LayoutLoad = ({ url, data }) => ({
  ...data,
  locale: localeOfPath(url.pathname) ?? DEFAULT_LOCALE,
});
