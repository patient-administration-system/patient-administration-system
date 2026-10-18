//! Per-user authentication for the REST API (DTAC-4), built on the published
//! [`authentication-verifier`](https://crates.io/crates/authentication-verifier) crate.
//!
//! Until now the API trusted one shared bearer token, and recorded in the audit log whatever
//! `X-User-Id` the caller chose to send. That cannot say who cancelled an appointment. With this
//! module a caller presents a short-lived **PASETO v4.public** token; it is verified offline against
//! the authentication-service's published Ed25519 keys, and two things follow from the *verified*
//! claims:
//!
//! - **Who:** the audit user id is the token's `sub`. Any client-supplied `X-User-Id` is discarded.
//! - **What they may do:** the crate's ABAC [`Policy`] decides from the token's attributes (by
//!   default: `svc=true` anything, `access=admin` destructive and write, `access=write` write,
//!   otherwise read-only).
//!
//! A trusted *service* (token attribute `svc=true`), such as the UI's backend-for-frontend, may say
//! which person it is acting for with `X-On-Behalf-Of`. The audit id then reads
//! `svc:<service> for <person>`: the person is the service's assertion, but the service is
//! authenticated. Any other caller's `X-On-Behalf-Of` is ignored, so a user cannot impersonate.
//!
//! # Modes (environment)
//!
//! | Configured | Behaviour |
//! |---|---|
//! | `PAS_PASETO_KEYS_URL` or `PAS_PASETO_KEYS_JSON` | A PASETO token is required. The shared `API_TOKEN` is **refused** unless `PAS_ALLOW_LEGACY_API_TOKEN=true`. |
//! | only `API_TOKEN` | The old behaviour: one shared token, identity unverified. |
//! | nothing | Trusted-caller mode, as before; the server logs a warning. |
//!
//! Message carriers posting replies to `/api/write-back/*` authenticate with their own signed credential
//! instead (`PAS_CARRIER_SECRETS`, see [`super::carrier_auth`]); they can reach nothing else.
//!
//! `PAS_REQUIRE_AUTH=true` makes the server **refuse to start** unless per-user (PASETO) auth is
//! configured. Use it in every real deployment.
//!
//! `/api/health` is always open. `/api/upstream/*` (the signed merge webhooks) is exempt from the
//! bearer check in PASETO mode because those peers authenticate by HMAC signature, which their
//! handlers verify.

use std::sync::Arc;
use std::time::Duration;

use authentication_verifier::{Action, Claims, Policy, ReloadableVerifier, Verifier};
use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::api::ApiResponse;
use crate::api::rest::carrier_auth::{self, Carriers};

/// A trusted service names the person it acts for with this header.
pub const ON_BEHALF_OF: &str = "x-on-behalf-of";
const USER_ID: &str = "x-user-id";
const MAX_IDENTITY: usize = 200;

/// Where the settings come from. Pure data, so the rules are testable without the environment.
#[derive(Debug, Clone, Default)]
pub struct UserAuthConfig {
    pub keys_url: Option<String>,
    pub keys_json: Option<String>,
    pub issuer: String,
    pub audience: String,
    /// The old shared token (`API_TOKEN`).
    pub legacy_token: Option<String>,
    pub allow_legacy: bool,
    pub require_auth: bool,
    /// `PAS_CARRIER_SECRETS`: one signing secret per message carrier (PILOT-3).
    pub carrier_secrets: Option<String>,
}

impl UserAuthConfig {
    pub fn from_env(legacy_token: Option<String>) -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let truthy = |k: &str| {
            get(k).is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        };
        Self {
            keys_url: get("PAS_PASETO_KEYS_URL"),
            keys_json: get("PAS_PASETO_KEYS_JSON"),
            issuer: get("PAS_PASETO_ISSUER").unwrap_or_else(|| "authentication-service".into()),
            audience: get("PAS_PASETO_AUDIENCE")
                .unwrap_or_else(|| "patient-administration-system".into()),
            legacy_token,
            allow_legacy: truthy("PAS_ALLOW_LEGACY_API_TOKEN"),
            require_auth: truthy("PAS_REQUIRE_AUTH"),
            carrier_secrets: get("PAS_CARRIER_SECRETS"),
        }
    }

    fn paseto_configured(&self) -> bool {
        self.keys_url.is_some() || self.keys_json.is_some()
    }
}

/// Who a request is from, once authenticated.
#[derive(Debug, Clone)]
pub enum Principal {
    /// The old shared token. Identity is whatever the caller said.
    Legacy,
    /// A verified token.
    Verified(Box<Claims>),
}

/// Why a request was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    /// No usable credential: 401.
    Unauthenticated(&'static str),
    /// A valid credential the policy does not allow: 403.
    Forbidden(String),
}

pub struct UserAuth {
    paseto: Option<ReloadableVerifier>,
    policy: Policy,
    legacy: Option<String>,
    carriers: Carriers,
    keys_url: Option<String>,
    issuer: String,
    audience: String,
}

impl UserAuth {
    /// Decide the mode from the configuration. `Ok(None)` is trusted-caller mode. An `Err` is a
    /// configuration the server must not start with.
    pub async fn build(cfg: UserAuthConfig) -> Result<Option<Arc<UserAuth>>, String> {
        if cfg.require_auth && !cfg.paseto_configured() {
            return Err("PAS_REQUIRE_AUTH is set but per-user authentication is not configured: set \
                        PAS_PASETO_KEYS_URL (or PAS_PASETO_KEYS_JSON). A shared API_TOKEN alone does not \
                        say who acted."
                .into());
        }
        let paseto = if let Some(json) = &cfg.keys_json {
            let value: serde_json::Value = serde_json::from_str(json)
                .map_err(|e| format!("PAS_PASETO_KEYS_JSON is not JSON: {e}"))?;
            Some(
                Verifier::from_paseto_keys_value(&value, &cfg.issuer, &cfg.audience)
                    .map_err(|e| format!("PAS_PASETO_KEYS_JSON: {e}"))?,
            )
        } else if let Some(url) = &cfg.keys_url {
            match Verifier::from_paseto_keys_url(url, &cfg.issuer, &cfg.audience).await {
                Ok(v) => Some(v),
                Err(e) => {
                    // Fail closed but keep booting: an empty key set rejects every token, and the
                    // refresh task picks the keys up as soon as the auth service is reachable.
                    tracing::error!(target: "pas::auth", "could not fetch PASETO keys from {url}: {e}; rejecting all tokens until they load");
                    Some(
                        Verifier::from_paseto_keys_value(
                            &serde_json::json!({ "keys": [] }),
                            &cfg.issuer,
                            &cfg.audience,
                        )
                        .map_err(|e| e.to_string())?,
                    )
                }
            }
        } else {
            None
        };
        let carriers = match &cfg.carrier_secrets {
            Some(spec) => Carriers::parse(spec)?,
            None => Carriers::default(),
        };
        let legacy_allowed = paseto.is_none() || cfg.allow_legacy;
        let legacy = cfg
            .legacy_token
            .clone()
            .filter(|t| !t.is_empty() && legacy_allowed);
        if paseto.is_none() && legacy.is_none() {
            return Ok(None);
        }
        if paseto.is_some() && legacy.is_some() {
            tracing::warn!(target: "pas::auth", "PAS_ALLOW_LEGACY_API_TOKEN is on: the shared token is accepted alongside per-user tokens, and its callers are not individually identified");
        }
        Ok(Some(Arc::new(UserAuth {
            paseto: paseto.map(ReloadableVerifier::new),
            policy: Policy::default_policy(),
            legacy,
            carriers,
            keys_url: cfg.keys_url,
            issuer: cfg.issuer,
            audience: cfg.audience,
        })))
    }

    /// True when per-user (PASETO) authentication is on.
    pub fn is_per_user(&self) -> bool {
        self.paseto.is_some()
    }

    /// Replace the key set (key rotation). Keeps the current keys and returns the error if the new
    /// document is bad.
    pub fn reload_keys(&self, keys: &serde_json::Value) -> Result<usize, String> {
        let rv = self
            .paseto
            .as_ref()
            .ok_or("per-user authentication is not configured")?;
        let v = Verifier::from_paseto_keys_value(keys, &self.issuer, &self.audience)
            .map_err(|e| e.to_string())?;
        let n = v.key_count();
        rv.store(v);
        Ok(n)
    }

    /// Check the credential. Never logs or echoes the token.
    pub fn authenticate(&self, headers: &HeaderMap) -> Result<Principal, Denied> {
        let token = bearer(headers).ok_or(Denied::Unauthenticated("missing bearer token"))?;
        if let Some(rv) = &self.paseto {
            match rv.current().verify(token) {
                Ok(claims) => return Ok(Principal::Verified(Box::new(claims))),
                Err(e) => tracing::debug!(target: "pas::auth", "token refused: {e}"),
            }
        }
        if let Some(expected) = &self.legacy
            && constant_time_eq(token.as_bytes(), expected.as_bytes())
        {
            return Ok(Principal::Legacy);
        }
        Err(Denied::Unauthenticated("invalid or expired token"))
    }

    /// Decide whether the principal may make this request.
    pub fn authorize(
        &self,
        principal: &Principal,
        method: &Method,
        path: &str,
    ) -> Result<(), Denied> {
        match principal {
            // The shared token has always been all-or-nothing.
            Principal::Legacy => Ok(()),
            Principal::Verified(claims) => {
                let d = self
                    .policy
                    .evaluate(claims, derive_action(method, path), &entity_of(path));
                if d.allowed {
                    Ok(())
                } else {
                    Err(Denied::Forbidden(d.reason))
                }
            }
        }
    }

    /// The paths that do not need a bearer token.
    fn is_open(&self, path: &str) -> bool {
        path == "/api/health" || (self.is_per_user() && path.starts_with("/api/upstream/"))
    }
}

/// The audit identity for a verified principal. A trusted service may name the person it acts for.
pub fn audit_identity(claims: &Claims, on_behalf_of: Option<&str>) -> String {
    let is_service = claims
        .attrs
        .get("svc")
        .is_some_and(|v| v.iter().any(|x| x == "true"));
    let who = match on_behalf_of.map(str::trim).filter(|s| !s.is_empty()) {
        Some(person) if is_service => format!("svc:{} for {}", claims.sub, person),
        _ => claims.sub.clone(),
    };
    who.chars()
        .filter(|c| !c.is_control())
        .take(MAX_IDENTITY)
        .collect()
}

/// `Authorization: Bearer <token>`; the scheme is case-insensitive (RFC 7235).
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The policy action for a request: reads, writes, deletes, and the named destructive POSTs.
pub fn derive_action(method: &Method, path: &str) -> Action {
    match *method {
        Method::GET | Method::HEAD | Method::OPTIONS => Action::Read,
        Method::DELETE => Action::Delete,
        _ if path.contains("/import")
            || path.contains("/merge")
            || path.contains("/dead-letters/") =>
        {
            Action::Destructive
        }
        _ => Action::Write,
    }
}

/// The entity a path addresses: `/api/appointments/1` is `appointments`, `/fhir/Patient/1` is `patient`.
pub fn entity_of(path: &str) -> String {
    let mut parts = path.trim_start_matches('/').split('/');
    let first = parts.next().unwrap_or("");
    let seg = if matches!(first, "api" | "fhir") {
        parts.next().unwrap_or("")
    } else {
        first
    };
    seg.to_ascii_lowercase()
}

fn refuse(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(ApiResponse::<serde_json::Value>::err(code, message)),
    )
        .into_response()
}

/// The middleware. Mount it with [`with_user_auth`].
pub async fn user_auth_middleware(
    State(auth): State<Arc<UserAuth>>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    if auth.is_open(&path) {
        return next.run(req).await;
    }
    // A carrier's signed reply (PILOT-3) is authenticated by its own HMAC credential, never by a bearer
    // token, and may reach the write-back API only.
    if path.starts_with("/api/write-back/") && req.headers().contains_key(carrier_auth::ID_HEADER) {
        return carrier_request(&auth, req, next).await;
    }
    let principal = match auth.authenticate(req.headers()) {
        Ok(p) => p,
        Err(Denied::Unauthenticated(msg)) => {
            return refuse(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg);
        }
        Err(Denied::Forbidden(m)) => return refuse(StatusCode::FORBIDDEN, "FORBIDDEN", &m),
    };
    if let Err(Denied::Forbidden(reason)) = auth.authorize(&principal, req.method(), &path) {
        tracing::info!(target: "pas::auth", "refused {} {path}: {reason}", req.method());
        return refuse(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "your account is not permitted to do that",
        );
    }
    if let Principal::Verified(claims) = &principal {
        // The caller's own claim about who they are is discarded; the verified subject replaces it.
        let behalf = req
            .headers()
            .get(ON_BEHALF_OF)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let id = audit_identity(claims, behalf.as_deref());
        req.headers_mut().remove(USER_ID);
        if let Ok(v) = HeaderValue::from_str(&id) {
            req.headers_mut().insert(USER_ID, v);
        }
    }
    next.run(req).await
}

/// Largest body a carrier may send (a 1000-row CSV batch is well under this).
const MAX_CARRIER_BODY: usize = 2 * 1024 * 1024;

async fn carrier_request(auth: &UserAuth, req: Request, next: Next) -> Response {
    let (mut parts, body) = req.into_parts();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let (Some(id), Some(ts), Some(sig)) = (
        header(carrier_auth::ID_HEADER),
        header(carrier_auth::TIMESTAMP_HEADER),
        header(carrier_auth::SIGNATURE_HEADER),
    ) else {
        return refuse(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid or expired credentials",
        );
    };
    let Ok(bytes) = axum::body::to_bytes(body, MAX_CARRIER_BODY).await else {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "TOO_LARGE",
            "request body too large",
        );
    };
    if let Err(e) = auth.carriers.verify(
        &id,
        &ts,
        &sig,
        &bytes,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    ) {
        // The reason is for the operator's log; the caller learns nothing about which part failed.
        tracing::warn!(target: "pas::auth", "carrier {id:?} refused: {e:?}");
        return refuse(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid or expired credentials",
        );
    }
    parts.headers.remove(USER_ID);
    if let Ok(v) = HeaderValue::from_str(&format!("carrier:{id}")) {
        parts.headers.insert(USER_ID, v);
    }
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

/// Put the authentication layer on a router.
pub fn with_user_auth(router: axum::Router, auth: Arc<UserAuth>) -> axum::Router {
    router.layer(axum::middleware::from_fn_with_state(
        auth,
        user_auth_middleware,
    ))
}

/// Re-fetch the key set on a timer so a key rotation needs no restart. On a failed fetch the
/// current keys stay in force.
pub fn spawn_key_refresh(auth: Arc<UserAuth>, every: Duration) {
    let Some(url) = auth.keys_url.clone() else {
        return;
    };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.tick().await; // the first tick is immediate; the keys were just loaded
        loop {
            tick.tick().await;
            match Verifier::from_paseto_keys_url(&url, &auth.issuer, &auth.audience).await {
                Ok(v) => {
                    if let Some(rv) = &auth.paseto {
                        rv.store(v);
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "pas::auth", "key refresh failed, keeping the current keys: {e}")
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(attrs: &[(&str, &str)]) -> Claims {
        let mut c = Claims {
            sub: "user-1".into(),
            email: String::new(),
            name: String::new(),
            iss: "i".into(),
            aud: "a".into(),
            exp: 0,
            iat: 0,
            nbf: None,
            sid: String::new(),
            scope: vec![],
            roles: vec![],
            attrs: Default::default(),
        };
        for (k, v) in attrs {
            c.attrs
                .entry((*k).to_string())
                .or_default()
                .push((*v).to_string());
        }
        c
    }

    #[test]
    fn test_actions_follow_the_method_with_destructive_named_posts() {
        assert_eq!(
            derive_action(&Method::GET, "/api/appointments"),
            Action::Read
        );
        assert_eq!(
            derive_action(&Method::POST, "/api/appointments/1/cancel"),
            Action::Write
        );
        assert_eq!(
            derive_action(&Method::PUT, "/api/letter-templates/1"),
            Action::Write
        );
        assert_eq!(
            derive_action(&Method::DELETE, "/api/standby/1"),
            Action::Delete
        );
        assert_eq!(
            derive_action(&Method::POST, "/api/patients/import"),
            Action::Destructive
        );
        assert_eq!(
            derive_action(&Method::POST, "/api/admin/outbox/dead-letters/1/replay"),
            Action::Destructive
        );
    }

    #[test]
    fn test_entity_is_the_first_segment_after_api_or_fhir() {
        assert_eq!(entity_of("/api/appointments/123"), "appointments");
        assert_eq!(entity_of("/fhir/Patient/9"), "patient");
        assert_eq!(entity_of("/api"), "");
        assert_eq!(entity_of("/swagger-ui/"), "swagger-ui");
    }

    #[test]
    fn test_bearer_scheme_is_case_insensitive_and_needs_a_token() {
        let h = |v: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::AUTHORIZATION, v.parse().unwrap());
            m
        };
        assert_eq!(bearer(&h("Bearer abc")), Some("abc"));
        assert_eq!(bearer(&h("bearer abc")), Some("abc"));
        assert_eq!(bearer(&h("BEARER  abc ")), Some("abc"));
        assert_eq!(bearer(&h("Basic abc")), None);
        assert_eq!(bearer(&h("Bearer ")), None);
        assert_eq!(bearer(&h("abc")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }

    #[test]
    fn test_constant_time_eq_is_exact() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secre"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn test_only_a_service_may_name_the_person_it_acts_for() {
        let user = claims(&[("access", "write")]);
        let svc = claims(&[("svc", "true")]);
        assert_eq!(
            audit_identity(&user, Some("someone-else")),
            "user-1",
            "a user cannot impersonate"
        );
        assert_eq!(
            audit_identity(&svc, Some("nurse-7")),
            "svc:user-1 for nurse-7"
        );
        assert_eq!(audit_identity(&svc, None), "user-1");
        assert_eq!(audit_identity(&svc, Some("  ")), "user-1");
        let long = "x".repeat(500);
        assert_eq!(audit_identity(&svc, Some(&long)).len(), MAX_IDENTITY);
        assert!(
            !audit_identity(&svc, Some("a\nb")).contains('\n'),
            "control characters are dropped"
        );
    }
}
