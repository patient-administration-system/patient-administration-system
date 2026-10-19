//! DTAC-4: per-user authentication with the published `authentication-verifier` crate.
//! No database needed: the real middleware is mounted on a tiny router that echoes the identity the
//! handlers would put in the audit log.

use authentication_verifier::Claims;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    routing::{get, post},
};
use base64::Engine;
use ed25519_dalek::SigningKey;
use patient_administration_system::api::rest::user_auth::{
    UserAuth, UserAuthConfig, with_user_auth,
};
use rusty_paseto::core::{Footer, Key, Paseto, PasetoAsymmetricPrivateKey, Payload, Public, V4};
use std::sync::Arc;
use tower::ServiceExt;

const ISSUER: &str = "authentication-service";
const AUDIENCE: &str = "patient-administration-system";
const KID: &str = "key-1";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn keys_json(seed: u8, kid: &str) -> String {
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(key(seed).verifying_key().to_bytes());
    serde_json::json!({ "keys": [{ "kty": "OKP", "crv": "Ed25519", "use": "sig", "kid": kid, "x": x }] }).to_string()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

struct Tok {
    seed: u8,
    kid: &'static str,
    iss: &'static str,
    aud: &'static str,
    exp_in: i64,
    attrs: Vec<(&'static str, &'static str)>,
}

impl Default for Tok {
    fn default() -> Self {
        Tok {
            seed: 1,
            kid: KID,
            iss: ISSUER,
            aud: AUDIENCE,
            exp_in: 300,
            attrs: vec![],
        }
    }
}

impl Tok {
    fn attr(mut self, k: &'static str, v: &'static str) -> Self {
        self.attrs.push((k, v));
        self
    }
    fn mint(&self) -> String {
        let mut attrs = std::collections::BTreeMap::<String, Vec<String>>::new();
        for (k, v) in &self.attrs {
            attrs.entry((*k).into()).or_default().push((*v).into());
        }
        let claims = Claims {
            sub: "11111111-1111-1111-1111-111111111111".into(),
            email: "nurse@example.test".into(),
            name: "Nurse".into(),
            iss: self.iss.into(),
            aud: self.aud.into(),
            exp: now() + self.exp_in,
            iat: now(),
            nbf: None,
            sid: "22222222-2222-2222-2222-222222222222".into(),
            scope: vec![],
            roles: vec![],
            attrs,
        };
        let kp = Key::<64>::from(key(self.seed).to_keypair_bytes());
        let private = PasetoAsymmetricPrivateKey::<V4, Public>::from(&kp);
        let footer = format!(r#"{{"kid":"{}"}}"#, self.kid);
        let mut b = Paseto::<V4, Public>::builder();
        let payload = serde_json::to_string(&claims).unwrap();
        b.set_payload(Payload::from(payload.as_str()));
        b.set_footer(Footer::from(footer.as_str()));
        b.try_sign(&private).unwrap()
    }
}

async fn echo(headers: HeaderMap) -> String {
    headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

async fn echo_body(headers: HeaderMap, body: String) -> String {
    format!(
        "{}|{}",
        headers
            .get("x-user-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-"),
        body
    )
}

fn app(auth: Arc<UserAuth>) -> Router {
    let r = Router::new()
        .route("/api/echo", get(echo).post(echo).put(echo).delete(echo))
        .route("/api/health", get(echo))
        .route("/api/write-back/replies", post(echo_body))
        .route("/api/inbound-texts/open", get(echo))
        .route("/api/upstream/person-merged", post(echo))
        .route("/api/patients/import", post(echo));
    with_user_auth(r, auth)
}

fn cfg() -> UserAuthConfig {
    UserAuthConfig {
        keys_json: Some(keys_json(1, KID)),
        issuer: ISSUER.into(),
        audience: AUDIENCE.into(),
        ..Default::default()
    }
}

async fn per_user() -> Arc<UserAuth> {
    UserAuth::build(cfg()).await.unwrap().unwrap()
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    extra: &[(&str, &str)],
) -> (StatusCode, String) {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    for (k, v) in extra {
        b = b.header(*k, *v);
    }
    let r = app
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let st = r.status();
    let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (st, String::from_utf8_lossy(&body).to_string())
}

#[tokio::test]
async fn every_kind_of_bad_token_is_refused_with_401_and_none_is_echoed_back() {
    let app = app(per_user().await);
    let good = Tok::default().mint();
    assert_eq!(
        call(&app, "GET", "/api/echo", Some(&good), &[]).await.0,
        StatusCode::OK,
        "control: a good token works"
    );

    let cases: Vec<(&str, Option<String>)> = vec![
        ("no token", None),
        ("garbage", Some("not-a-token".into())),
        (
            "a JWT-looking string",
            Some("eyJhbGciOiJSUzI1NiJ9.e30.sig".into()),
        ),
        (
            "expired",
            Some(
                Tok {
                    exp_in: -120,
                    ..Default::default()
                }
                .mint(),
            ),
        ),
        (
            "wrong audience",
            Some(
                Tok {
                    aud: "some-other-service",
                    ..Default::default()
                }
                .mint(),
            ),
        ),
        (
            "wrong issuer",
            Some(
                Tok {
                    iss: "evil-auth",
                    ..Default::default()
                }
                .mint(),
            ),
        ),
        (
            "signed by another key",
            Some(
                Tok {
                    seed: 9,
                    ..Default::default()
                }
                .mint(),
            ),
        ),
        (
            "unknown kid",
            Some(
                Tok {
                    kid: "other-key",
                    ..Default::default()
                }
                .mint(),
            ),
        ),
    ];
    for (name, token) in cases {
        let (st, body) = call(&app, "GET", "/api/echo", token.as_deref(), &[]).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "{name}");
        if let Some(t) = token {
            assert!(
                !body.contains(&t),
                "{name}: the token must never be echoed in an error"
            );
        }
    }
}

#[tokio::test]
async fn the_policy_decides_from_the_tokens_attributes() {
    let app = app(per_user().await);
    let none = Tok::default().mint();
    let write = Tok::default().attr("access", "write").mint();
    let admin = Tok::default().attr("access", "admin").mint();
    let svc = Tok::default().attr("svc", "true").mint();

    // No attributes: read-only.
    assert_eq!(
        call(&app, "GET", "/api/echo", Some(&none), &[]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/api/echo", Some(&none), &[]).await.0,
        StatusCode::FORBIDDEN
    );
    // access=write: may write, may not delete or do destructive things.
    assert_eq!(
        call(&app, "POST", "/api/echo", Some(&write), &[]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "PUT", "/api/echo", Some(&write), &[]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "DELETE", "/api/echo", Some(&write), &[]).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "POST", "/api/patients/import", Some(&write), &[])
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // access=admin: everything destructive too.
    assert_eq!(
        call(&app, "DELETE", "/api/echo", Some(&admin), &[]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/api/patients/import", Some(&admin), &[])
            .await
            .0,
        StatusCode::OK
    );
    // A service peer.
    assert_eq!(
        call(&app, "DELETE", "/api/echo", Some(&svc), &[]).await.0,
        StatusCode::OK
    );
    // A refusal says so without leaking the rule.
    let (_, body) = call(&app, "POST", "/api/echo", Some(&none), &[]).await;
    assert!(
        body.contains("FORBIDDEN") && !body.contains("rule"),
        "{body}"
    );
}

#[tokio::test]
async fn the_audit_identity_is_the_verified_subject_and_a_forged_header_is_discarded() {
    let app = app(per_user().await);
    let t = Tok::default().attr("access", "write").mint();
    let (_, id) = call(
        &app,
        "POST",
        "/api/echo",
        Some(&t),
        &[("x-user-id", "ceo-who-did-nothing")],
    )
    .await;
    assert_eq!(
        id, "11111111-1111-1111-1111-111111111111",
        "the forged X-User-Id never reaches the handler"
    );
    let (_, id) = call(&app, "POST", "/api/echo", Some(&t), &[]).await;
    assert_eq!(id, "11111111-1111-1111-1111-111111111111");
}

#[tokio::test]
async fn only_an_authenticated_service_can_name_the_person_it_acts_for() {
    let app = app(per_user().await);
    let svc = Tok::default().attr("svc", "true").mint();
    let user = Tok::default().attr("access", "write").mint();
    let (_, id) = call(
        &app,
        "POST",
        "/api/echo",
        Some(&svc),
        &[("x-on-behalf-of", "dr-patel")],
    )
    .await;
    assert_eq!(id, "svc:11111111-1111-1111-1111-111111111111 for dr-patel");
    let (_, id) = call(
        &app,
        "POST",
        "/api/echo",
        Some(&user),
        &[("x-on-behalf-of", "dr-patel")],
    )
    .await;
    assert_eq!(
        id, "11111111-1111-1111-1111-111111111111",
        "a user cannot impersonate"
    );
}

#[tokio::test]
async fn health_is_open_and_webhooks_are_open_only_in_per_user_mode() {
    let app = app(per_user().await);
    assert_eq!(
        call(&app, "GET", "/api/health", None, &[]).await.0,
        StatusCode::OK
    );
    // Merge webhooks authenticate by HMAC in their own handler, so the bearer check steps aside.
    assert_eq!(
        call(&app, "POST", "/api/upstream/person-merged", None, &[])
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "GET", "/api/echo", None, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );

    // In the old shared-token mode the webhooks still needed the token, as before.
    let legacy = UserAuth::build(UserAuthConfig {
        legacy_token: Some("s3cret".into()),
        issuer: ISSUER.into(),
        audience: AUDIENCE.into(),
        ..Default::default()
    })
    .await
    .unwrap()
    .unwrap();
    let app = self::app(legacy);
    assert_eq!(
        call(&app, "POST", "/api/upstream/person-merged", None, &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn the_shared_token_is_refused_in_per_user_mode_unless_explicitly_allowed() {
    let strict = UserAuth::build(UserAuthConfig {
        legacy_token: Some("s3cret".into()),
        ..cfg()
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        call(&app(strict), "GET", "/api/echo", Some("s3cret"), &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "the shared token no longer works"
    );

    let lenient = UserAuth::build(UserAuthConfig {
        legacy_token: Some("s3cret".into()),
        allow_legacy: true,
        ..cfg()
    })
    .await
    .unwrap()
    .unwrap();
    let app = app(lenient);
    assert_eq!(
        call(&app, "DELETE", "/api/echo", Some("s3cret"), &[])
            .await
            .0,
        StatusCode::OK,
        "explicitly allowed, and all-or-nothing as ever"
    );
    assert_eq!(
        call(&app, "GET", "/api/echo", Some("s3cret-"), &[]).await.0,
        StatusCode::UNAUTHORIZED,
        "close is not equal"
    );
    let t = Tok::default().mint();
    assert_eq!(
        call(&app, "GET", "/api/echo", Some(&t), &[]).await.0,
        StatusCode::OK,
        "per-user tokens still work alongside"
    );
}

#[tokio::test]
async fn the_old_shared_token_mode_still_works_when_nothing_else_is_configured() {
    let auth = UserAuth::build(UserAuthConfig {
        legacy_token: Some("s3cret".into()),
        ..Default::default()
    })
    .await
    .unwrap()
    .unwrap();
    assert!(!auth.is_per_user());
    let app = app(auth);
    assert_eq!(
        call(&app, "GET", "/api/echo", Some("s3cret"), &[]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "GET", "/api/echo", Some("wrong"), &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "GET", "/api/echo", None, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Its identity is, as ever, whatever the caller said: that is exactly why this mode is weaker.
    let (_, id) = call(
        &app,
        "GET",
        "/api/echo",
        Some("s3cret"),
        &[("x-user-id", "claimed")],
    )
    .await;
    assert_eq!(id, "claimed");
}

#[tokio::test]
async fn startup_refuses_unsafe_configurations_and_allows_trusted_caller_mode_only_when_not_required()
 {
    // Nothing configured: trusted-caller mode, allowed.
    assert!(
        UserAuth::build(UserAuthConfig {
            issuer: ISSUER.into(),
            audience: AUDIENCE.into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .is_none()
    );
    // Required but nothing configured, or only the shared token: refuse to start.
    let e = UserAuth::build(UserAuthConfig {
        require_auth: true,
        ..Default::default()
    })
    .await
    .err()
    .expect("must refuse");
    assert!(e.contains("PAS_REQUIRE_AUTH"), "{e}");
    assert!(
        UserAuth::build(UserAuthConfig {
            require_auth: true,
            legacy_token: Some("x".into()),
            ..Default::default()
        })
        .await
        .is_err(),
        "a shared token does not say who acted"
    );
    // Required and configured: fine.
    assert!(
        UserAuth::build(UserAuthConfig {
            require_auth: true,
            ..cfg()
        })
        .await
        .unwrap()
        .is_some()
    );
    // A key document that is not valid is a startup error, not a silent open door.
    assert!(
        UserAuth::build(UserAuthConfig {
            keys_json: Some("not json".into()),
            ..cfg()
        })
        .await
        .is_err()
    );
    assert!(
        UserAuth::build(UserAuthConfig {
            keys_json: Some(r#"{"nokeys":true}"#.into()),
            ..cfg()
        })
        .await
        .is_err()
    );
}

#[tokio::test]
async fn rotating_the_keys_takes_effect_without_a_restart_and_a_bad_document_changes_nothing() {
    let auth = per_user().await;
    let old = Tok::default().mint();
    let new = Tok {
        seed: 2,
        kid: "key-2",
        ..Default::default()
    }
    .mint();
    let app = app(auth.clone());
    assert_eq!(
        call(&app, "GET", "/api/echo", Some(&old), &[]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "GET", "/api/echo", Some(&new), &[]).await.0,
        StatusCode::UNAUTHORIZED,
        "not yet published"
    );

    assert!(
        auth.reload_keys(&serde_json::json!({ "nokeys": true }))
            .is_err()
    );
    assert_eq!(
        call(&app, "GET", "/api/echo", Some(&old), &[]).await.0,
        StatusCode::OK,
        "a bad document leaves the current keys in force"
    );

    let doc: serde_json::Value = serde_json::from_str(&keys_json(2, "key-2")).unwrap();
    assert_eq!(auth.reload_keys(&doc).unwrap(), 1);
    assert_eq!(
        call(&app, "GET", "/api/echo", Some(&new), &[]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "GET", "/api/echo", Some(&old), &[]).await.0,
        StatusCode::UNAUTHORIZED,
        "the retired key no longer verifies"
    );
}

const SECRET: &str = "carrier-secret-0123456789";

async fn carrier_app() -> Router {
    let auth = UserAuth::build(UserAuthConfig {
        carrier_secrets: Some(format!("twilio={SECRET},wa=other-secret-0123456789")),
        ..cfg()
    })
    .await
    .unwrap()
    .unwrap();
    app(auth)
}

async fn carrier_post(
    app: &Router,
    path: &str,
    id: &str,
    ts: i64,
    sig: &str,
    body: &str,
) -> (StatusCode, String) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("x-carrier-id", id)
                .header("x-carrier-timestamp", ts.to_string())
                .header("x-carrier-signature", sig)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let st = r.status();
    (
        st,
        String::from_utf8_lossy(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap())
            .to_string(),
    )
}

#[tokio::test]
async fn a_carrier_signed_reply_is_accepted_and_audited_under_the_carrier() {
    use patient_administration_system::api::rest::carrier_auth::sign;
    let app = carrier_app().await;
    let body = r#"{"message_id":"m1","action":"confirm"}"#;
    let ts = now();
    let (st, out) = carrier_post(
        &app,
        "/api/write-back/replies",
        "twilio",
        ts,
        &sign(SECRET.as_bytes(), ts, body.as_bytes()),
        body,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        out,
        format!("carrier:twilio|{body}"),
        "the handler sees the intact body and the carrier identity, not anything the sender claimed"
    );
}

#[tokio::test]
async fn a_carrier_cannot_forge_replay_or_wander() {
    use patient_administration_system::api::rest::carrier_auth::sign;
    let app = carrier_app().await;
    let body = r#"{"message_id":"m1","action":"cancel"}"#;
    let ts = now();
    let good = sign(SECRET.as_bytes(), ts, body.as_bytes());
    // Tampered body, wrong secret, another carrier's id, unknown carrier, stale timestamp, no signature.
    for (name, id, t, sig, b) in [
        (
            "tampered body",
            "twilio",
            ts,
            good.clone(),
            r#"{"message_id":"m1","action":"confirm"}"#,
        ),
        (
            "wrong secret",
            "twilio",
            ts,
            sign(b"not-the-secret-0123456789", ts, body.as_bytes()),
            body,
        ),
        ("another carrier's id", "wa", ts, good.clone(), body),
        ("unknown carrier", "evil", ts, good.clone(), body),
        (
            "stale",
            "twilio",
            ts - 3600,
            sign(SECRET.as_bytes(), ts - 3600, body.as_bytes()),
            body,
        ),
        (
            "future",
            "twilio",
            ts + 3600,
            sign(SECRET.as_bytes(), ts + 3600, body.as_bytes()),
            body,
        ),
        ("no signature", "twilio", ts, String::new(), body),
    ] {
        let (st, out) = carrier_post(&app, "/api/write-back/replies", id, t, &sig, b).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "{name}");
        assert!(
            !out.contains(SECRET) && !out.contains("stale") && !out.contains("signature"),
            "{name}: says nothing about why: {out}"
        );
    }
    // A valid carrier credential opens nothing but the write-back API.
    let (st, _) = carrier_post(
        &app,
        "/api/echo",
        "twilio",
        ts,
        &sign(SECRET.as_bytes(), ts, body.as_bytes()),
        body,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "a carrier cannot reach other endpoints"
    );
    // Nor can it read the staff list of replies, or use any method but POST on its own endpoints.
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/inbound-texts/open")
                .header("x-carrier-id", "twilio")
                .header("x-carrier-timestamp", ts.to_string())
                .header("x-carrier-signature", sign(SECRET.as_bytes(), ts, b""))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        StatusCode::UNAUTHORIZED,
        "a carrier cannot read the staff list"
    );
    // A forged X-User-Id from the carrier is discarded in favour of its own id.
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/write-back/replies")
                .header("x-carrier-id", "twilio")
                .header("x-carrier-timestamp", ts.to_string())
                .header("x-carrier-signature", sign(SECRET.as_bytes(), ts, b"x"))
                .header("x-user-id", "admin")
                .body(Body::from("x"))
                .unwrap(),
        )
        .await
        .unwrap();
    let out = String::from_utf8_lossy(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap())
        .to_string();
    assert_eq!(out, "carrier:twilio|x");
}

#[tokio::test]
async fn weak_or_duplicate_carrier_secrets_stop_start_up() {
    for spec in [
        "twilio=short",
        "twilio=carrier-secret-0123456789,twilio=carrier-secret-9876543210",
        "noequals",
    ] {
        assert!(
            UserAuth::build(UserAuthConfig {
                carrier_secrets: Some(spec.into()),
                ..cfg()
            })
            .await
            .is_err(),
            "{spec}"
        );
    }
}
