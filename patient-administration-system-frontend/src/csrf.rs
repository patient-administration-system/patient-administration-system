//! CSRF protection — double-submit cookie pattern.
//!
//! Threat model: a third-party site tricks the admin's browser into
//! submitting one of the write-flow forms (admit, book, compose). We
//! defend with the standard double-submit-cookie pattern:
//!
//! 1. On every form GET, [`ensure_token`] reads (or, if absent,
//!    generates) a `pas_csrf` cookie with `SameSite=Strict; HttpOnly`
//!    (and `Secure` when [`cookie_secure`] returns true).
//! 2. The form embeds the same token as a hidden `csrf_token` field
//!    (server-side render via Tera).
//! 3. On submit, [`verify_token`] requires the cookie value and the
//!    posted form value to match (constant-time compare).
//!
//! `SameSite=Strict` + `HttpOnly` mean a third-party site can neither
//! read the cookie nor cause the browser to send it on a cross-origin
//! POST, so the attacker can't supply a matching `csrf_token` field.
//! The `Secure` flag (on by default wherever production-like) blocks the
//! cookie from ever travelling over plaintext HTTP.

use axa::{Environment, Error, Result};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use uuid::Uuid;

/// Name of the cookie that carries the CSRF token.
pub const COOKIE_NAME: &str = "pas_csrf";

/// Name of the form field every protected form must include.
pub const FORM_FIELD: &str = "csrf_token";

/// Env var that overrides the `Secure` cookie flag. Left out, the flag follows the environment: on anywhere
/// production-like, so a deployment that forgets it is not sending the token in the clear, and off in
/// development and test, so plaintext-HTTP localhost still works.
pub const SECURE_ENV: &str = "PAS_COOKIE_SECURE";

/// True iff the `Secure` cookie flag should be set on `pas_csrf`.
pub fn cookie_secure(environment: &Environment) -> bool {
    secure_for(environment, std::env::var(SECURE_ENV).ok().as_deref())
}

/// `setting` decides when it says yes or no. Anything else, a typing mistake included, leaves the environment's
/// default in place, so a mistake cannot take the flag away.
fn secure_for(environment: &Environment, setting: Option<&str>) -> bool {
    match setting {
        Some("1" | "true" | "TRUE" | "yes" | "YES") => true,
        Some("0" | "false" | "FALSE" | "no" | "NO") => false,
        _ => environment.is_production_like(),
    }
}

/// Return the CSRF token attached to this jar, generating + setting a
/// fresh one if no cookie is present. The returned [`CookieJar`] must be
/// returned to axum from the handler so the `Set-Cookie` header reaches
/// the browser.
pub fn ensure_token(jar: CookieJar, environment: &Environment) -> (String, CookieJar) {
    if let Some(c) = jar.get(COOKIE_NAME) {
        let value = c.value().to_string();
        if !value.is_empty() {
            return (value, jar);
        }
    }
    let token = Uuid::new_v4().simple().to_string();
    let cookie = Cookie::build((COOKIE_NAME, token.clone()))
        .path("/")
        .http_only(true)
        .secure(cookie_secure(environment))
        .same_site(SameSite::Strict)
        .build();
    (token, jar.add(cookie))
}

/// Validate that the supplied form `csrf_token` matches the cookie value.
/// Returns [`Error::BadRequest`] on mismatch.
pub fn verify_token(jar: &CookieJar, form_token: &str) -> Result<()> {
    let cookie_value = jar
        .get(COOKIE_NAME)
        .map(|c| c.value().to_string())
        .unwrap_or_default();
    if cookie_value.is_empty() || !constant_time_eq(cookie_value.as_bytes(), form_token.as_bytes())
    {
        return Err(Error::BadRequest("CSRF token mismatch".into()));
    }
    Ok(())
}

/// Byte-by-byte equality without short-circuiting on length or content.
/// Length mismatch is a fast-fail (which leaks the length, but the token
/// length is a constant so that's fine).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq_match() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_constant_time_eq_mismatch() {
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abc", b""));
    }

    #[test]
    fn test_ensure_token_generates_when_missing() {
        let jar = CookieJar::new();
        let (token, jar) = ensure_token(jar, &Environment::Test);
        assert_eq!(token.len(), 32, "uuid simple format is 32 hex chars");
        assert_eq!(
            jar.get(COOKIE_NAME).map(Cookie::value),
            Some(token.as_str())
        );
    }

    #[test]
    fn test_ensure_token_reuses_existing() {
        let jar = CookieJar::new().add(
            Cookie::build((COOKIE_NAME, "existing-token-value"))
                .path("/")
                .build(),
        );
        let (token, _jar) = ensure_token(jar, &Environment::Test);
        assert_eq!(token, "existing-token-value");
    }

    #[test]
    fn test_verify_token_accepts_match() {
        let jar = CookieJar::new().add(Cookie::build((COOKIE_NAME, "abc123")).path("/").build());
        assert!(verify_token(&jar, "abc123").is_ok());
    }

    #[test]
    fn test_verify_token_rejects_mismatch() {
        let jar = CookieJar::new().add(Cookie::build((COOKIE_NAME, "abc123")).path("/").build());
        assert!(verify_token(&jar, "abc124").is_err());
    }

    #[test]
    fn test_verify_token_rejects_missing_cookie() {
        let jar = CookieJar::new();
        assert!(verify_token(&jar, "abc123").is_err());
    }

    #[test]
    fn test_verify_token_rejects_empty_form() {
        let jar = CookieJar::new().add(Cookie::build((COOKIE_NAME, "abc123")).path("/").build());
        assert!(verify_token(&jar, "").is_err());
    }

    #[test]
    fn the_secure_flag_follows_the_environment_unless_told() {
        let staging = Environment::Other("staging".to_string());
        for production in [&Environment::Production, &staging] {
            assert!(secure_for(production, None), "{production}: on by default");
            assert!(
                !secure_for(production, Some("0")),
                "{production}: a plain no"
            );
            assert!(
                secure_for(production, Some("flase")),
                "{production}: a typing mistake does not turn it off"
            );
        }
        for local in [&Environment::Development, &Environment::Test] {
            assert!(!secure_for(local, None), "{local}: off by default");
            assert!(secure_for(local, Some("1")), "{local}: a plain yes");
        }
    }

    #[test]
    fn a_new_cookie_in_production_carries_the_secure_flag() {
        let (_token, jar) = ensure_token(CookieJar::new(), &Environment::Production);
        let cookie = jar.get(COOKIE_NAME).expect("cookie must be set");
        // `PAS_COOKIE_SECURE=0` in the environment running the tests would be a plain no.
        if std::env::var(SECURE_ENV).is_err() {
            assert_eq!(cookie.secure(), Some(true));
        }
    }
}
