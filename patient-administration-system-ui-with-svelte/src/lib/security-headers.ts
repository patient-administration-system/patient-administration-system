// Response security headers, applied to every response by `handle` in
// `src/hooks.server.ts`. The Content-Security-Policy is configured
// separately (`csp` in `vite.config.ts`) because SvelteKit must add its own
// nonces/hashes to it.

/** Sent on every response, over HTTP and HTTPS alike. */
export const SECURITY_HEADERS: Readonly<Record<string, string>> = {
  "X-Content-Type-Options": "nosniff",
  "X-Frame-Options": "DENY",
  "Referrer-Policy": "strict-origin-when-cross-origin",
  "Permissions-Policy": [
    "accelerometer=()",
    "autoplay=()",
    "bluetooth=()",
    "camera=()",
    "display-capture=()",
    "geolocation=()",
    "gyroscope=()",
    "hid=()",
    "magnetometer=()",
    "microphone=()",
    "midi=()",
    "payment=()",
    "serial=()",
    "usb=()",
  ].join(", "),
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Resource-Policy": "same-origin",
};

/** Browsers ignore HSTS over plain HTTP; sending it there is wrong. */
export const HSTS_VALUE = "max-age=31536000; includeSubDomains";

/** True when the request reached us over HTTPS, directly or via a proxy. */
export function isHttps(url: URL, forwardedProto: string | null): boolean {
  if (url.protocol === "https:") return true;
  const first = forwardedProto?.split(",")[0]?.trim().toLowerCase();
  return first === "https";
}

/**
 * Adds the security headers to `headers`, never overwriting one the route
 * already set. HSTS is added only for HTTPS requests.
 */
export function applySecurityHeaders(
  headers: Headers,
  https: boolean,
): Headers {
  const entries = https
    ? { "Strict-Transport-Security": HSTS_VALUE, ...SECURITY_HEADERS }
    : SECURITY_HEADERS;
  for (const [name, value] of Object.entries(entries)) {
    if (!headers.has(name)) headers.set(name, value);
  }
  return headers;
}
