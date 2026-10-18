# CSRF middleware

Reference for `src/csrf.rs`, the double-submit-cookie pattern that guards the three POST routes.

## Threat model

A third-party site tricks the admin's browser into submitting one of the write-flow forms (admit / book / compose letter). Without protection, the browser would happily attach the admin's session cookie to a cross-origin POST and the request would land as the admin.

Defence:

1. On every form **GET**, the handler reads (or mints) a `pas_csrf` cookie. The token is also embedded as a hidden `csrf_token` form field.
2. On **POST**, the handler requires the cookie and the form field to match in constant time. Any mismatch → HTTP **400** before any side effects.

`SameSite=Strict` + `HttpOnly` mean a third-party site can neither read the cookie nor cause the browser to send it on a cross-origin POST, so an attacker has no way to supply a matching `csrf_token` field.

## Public API

```rust
pub const COOKIE_NAME: &str = "pas_csrf";
pub const FORM_FIELD:  &str = "csrf_token";
pub const SECURE_ENV:  &str = "PAS_COOKIE_SECURE";

pub fn cookie_secure() -> bool;
pub fn ensure_token(jar: CookieJar) -> (String, CookieJar);
pub fn verify_token(jar: &CookieJar, form_token: &str) -> Result<()>;
```

### `ensure_token(jar) -> (String, CookieJar)`

Reads the `pas_csrf` cookie from the inbound jar. If present *and non-empty*, returns it unchanged. Otherwise mints a fresh `Uuid::new_v4().simple()` (32 hex chars), adds the cookie to the jar, and returns the new value.

Cookie attributes on the mint path:

| Attribute | Value | Why |
|---|---|---|
| `Path` | `/` | Cookie applies to every route on the host |
| `HttpOnly` | `true` | JavaScript on the same site cannot read it |
| `SameSite` | `Strict` | Browser does not send the cookie on cross-origin requests |
| `Secure` | `cookie_secure()` (see below) | HTTPS-only when truthy; off in dev |

The handler must thread the returned `CookieJar` back into the response (Loco's `IntoResponse` impl for `CookieJar` is what writes the `Set-Cookie` header).

### `verify_token(&jar, form_token) -> Result<()>`

Reads the `pas_csrf` cookie value from the jar. Returns `Err(Error::BadRequest("CSRF token mismatch"))` (which Loco maps to HTTP 400) when:

- The cookie is missing.
- The cookie value is empty.
- The cookie value does not match the form token byte-for-byte (constant-time XOR; length-checked first).

### `cookie_secure() -> bool`

Returns `true` iff `PAS_COOKIE_SECURE` is set to `1` / `true` / `TRUE` / `yes` / `YES`. Anything else (including unset, empty string, `0`, `false`) returns `false`.

Read on every call (env access is cheap) so tests can flip the variable mid-process to assert both states.

## Constant-time compare

```rust
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}
```

The length-mismatch early-return leaks the length, but the token length is a constant (`32` for `Uuid::new_v4().simple()`), so the leak is harmless.

## Operational notes

- **Dev**: leave `PAS_COOKIE_SECURE` unset. Plaintext-HTTP localhost works.
- **Production**: set `PAS_COOKIE_SECURE=1`. The cookie will not be sent over plaintext HTTP, which prevents accidental leakage if a misconfigured proxy serves the form over HTTP.
- **Cookie rotation**: not implemented in v0.1.0. The token is stable for the lifetime of the cookie (browser-controlled; no server-side expiry header is set). A logout / rotate flow would simply `Set-Cookie: pas_csrf=; Max-Age=0`.
- **Session binding**: not implemented in v0.1.0 (no sessions exist). When sessions land, bind the CSRF token to the session id (e.g. derive `HMAC(session_id, "csrf")`) so a stolen cookie alone is not enough.

## Tests

9 unit tests in `src/csrf.rs::tests`:

- `test_constant_time_eq_match` — equal slices, including empty.
- `test_constant_time_eq_mismatch` — same-length-different-bytes, different lengths, empty vs non-empty.
- `test_ensure_token_generates_when_missing` — fresh jar → 32-char token + cookie set.
- `test_ensure_token_reuses_existing` — pre-set cookie → returned unchanged.
- `test_verify_token_accepts_match` — cookie and form match.
- `test_verify_token_rejects_mismatch` — cookie and form differ by one byte.
- `test_verify_token_rejects_missing_cookie` — empty jar.
- `test_verify_token_rejects_empty_form` — cookie present, form value empty.
- `test_ensure_token_sets_secure_when_env_truthy` — saves + restores `PAS_COOKIE_SECURE` to stay independent of other concurrent tests.

Integration tests in `tests/dashboard_smoke.rs` also assert end-to-end that GET sets the cookie and POST without a valid token returns 400. See [testing.md](testing.md).

## See Also

- [architecture.md](architecture.md) — where CSRF sits in the request pipeline
- [routes.md](routes.md) — which routes are CSRF-protected
- [../README.md](../README.md#security--csrf-on-write-forms) — public-facing CSRF summary
- [../spec.md](../spec.md#43-write-forms--csrf-contract) — CSRF contract inside the spec
