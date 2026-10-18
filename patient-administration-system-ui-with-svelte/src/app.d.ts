// See https://svelte.dev/docs/kit/types#app
declare global {
  namespace App {
    interface Error {
      code?: string;
    }
    // BFF: the server holds the opaque OIDC session id from the httpOnly
    // `__Host-pas_oidc` cookie (resolved in `hooks.server.ts`); the browser
    // never sees a token.
    interface Locals {
      oidc: import("#lib/server/oidc-session.js").OidcSession | null;
      // The locale of this request: from the URL prefix, else negotiated. Always set by `hooks.server.ts`.
      locale: import("#lib/locales.js").Locale;
    }
    interface PageData {
      title?: string;
      locale?: import("#lib/locales.js").Locale;
    }
    interface PageState {}
    interface Platform {}
  }
}

export {};
