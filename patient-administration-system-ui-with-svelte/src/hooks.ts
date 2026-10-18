import type { Reroute } from "@sveltejs/kit/hooks";
import { splitPath } from "#lib/locales.js";

// Locale-prefixed URLs (`/cy-001/appointments`) are served by the routes that already exist
// (`/appointments`): strip a supported locale prefix before route matching. Only the matched route
// changes; `url`, links and `page.url` keep the prefix. An unprefixed path (including a two-letter
// one such as `/en/appointments`, which is not a locale) is left alone here; the server's `handle`
// redirects it before any page is rendered.
export const reroute: Reroute = ({ url }) => {
  const { locale, rest } = splitPath(url.pathname);
  return locale ? rest : undefined;
};
