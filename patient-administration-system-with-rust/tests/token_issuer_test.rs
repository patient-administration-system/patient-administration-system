//! DTAC-4, the issuing half: the real `pas-token-issuer` binary generates keys, mints a token, and
//! serves its public keys over HTTP; the API's real middleware fetches those keys from the URL and
//! accepts the token. Nothing here is mocked except the one handler behind the middleware.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use patient_administration_system::api::rest::user_auth::{
    UserAuth, UserAuthConfig, with_user_auth,
};
use tower::ServiceExt;

const BIN: &str = env!("CARGO_BIN_EXE_pas-token-issuer");

fn issuer(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("run pas-token-issuer")
}

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn app(auth: std::sync::Arc<UserAuth>) -> Router {
    with_user_auth(
        Router::new().route(
            "/api/echo",
            get(|h: axum::http::HeaderMap| async move {
                h.get("x-user-id")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_string()
            }),
        ),
        auth,
    )
}

async fn status(app: &Router, token: &str) -> (StatusCode, String) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
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
async fn the_api_fetches_the_issuers_keys_over_http_and_accepts_its_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_str().unwrap();
    let out = issuer(&["keygen", "--dir", d, "--kid", "k1"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The private key is owner-only; the public key set is publishable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("private.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "the private key must not be readable by others"
        );
    }
    let public = std::fs::read_to_string(dir.path().join("keys.json")).unwrap();
    assert!(
        public.contains("\"kid\": \"k1\"") && !public.contains("private"),
        "{public}"
    );

    let port = free_port();
    let token_file = dir.path().join("service.token");
    let _server = Server(
        Command::new(BIN)
            .args([
                "serve",
                "--key",
                &format!("{d}/private.key"),
                "--kid",
                "k1",
                "--sub",
                "pas-ui",
                "--ttl",
                "60",
                "--token-file",
                token_file.to_str().unwrap(),
                "--bind",
                &format!("127.0.0.1:{port}"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start issuer"),
    );
    let url = format!("http://127.0.0.1:{port}/keys.json");
    let deadline = Instant::now() + Duration::from_secs(15);
    let auth = loop {
        // The same code path as production: fetch the keys by URL at start-up.
        let cfg = UserAuthConfig {
            keys_url: Some(url.clone()),
            issuer: "authentication-service".into(),
            audience: "patient-administration-system".into(),
            ..Default::default()
        };
        let a = UserAuth::build(cfg).await.unwrap().unwrap();
        let probe = std::fs::read_to_string(&token_file).unwrap_or_default();
        if !probe.is_empty() && status(&app(a.clone()), probe.trim()).await.0 == StatusCode::OK {
            break a;
        }
        assert!(Instant::now() < deadline, "the issuer never became usable");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let app = app(auth);

    // The service token the issuer keeps in the file works, and identifies the service.
    let file_token = std::fs::read_to_string(&token_file).unwrap();
    let (st, id) = status(&app, file_token.trim()).await;
    assert_eq!((st, id.as_str()), (StatusCode::OK, "pas-ui"));

    // A token minted by hand with the same key works too, with the attributes asked for.
    let minted = issuer(&[
        "mint",
        "--key",
        &format!("{d}/private.key"),
        "--kid",
        "k1",
        "--sub",
        "ops-1",
        "--attr",
        "access=write",
    ]);
    let minted = String::from_utf8(minted.stdout).unwrap();
    assert_eq!(
        status(&app, minted.trim()).await,
        (StatusCode::OK, "ops-1".into())
    );

    // Another issuer's key (a fresh keygen) is not trusted, and an unknown kid is not either.
    let other = tempfile::tempdir().unwrap();
    issuer(&[
        "keygen",
        "--dir",
        other.path().to_str().unwrap(),
        "--kid",
        "k1",
    ]);
    let forged = issuer(&[
        "mint",
        "--key",
        other.path().join("private.key").to_str().unwrap(),
        "--kid",
        "k1",
        "--sub",
        "attacker",
        "--attr",
        "svc=true",
    ]);
    assert_eq!(
        status(&app, String::from_utf8(forged.stdout).unwrap().trim())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn the_issuer_refuses_to_mint_unsafe_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_str().unwrap();
    issuer(&["keygen", "--dir", d]);
    let key = format!("{d}/private.key");
    for bad in [
        &["--ttl", "100000"][..],
        &["--ttl", "0"],
        &["--ttl", "abc"],
        &["--attr", "novalue"],
    ] {
        let mut a = vec!["mint", "--key", &key, "--kid", "key-1", "--sub", "x"];
        a.extend_from_slice(bad);
        assert!(!issuer(&a).status.success(), "{bad:?} must be refused");
    }
    assert!(
        !issuer(&["mint", "--key", &key, "--kid", "key-1", "--sub", " "])
            .status
            .success(),
        "a subject is required"
    );
    assert!(
        !issuer(&["mint", "--key", "/nonexistent", "--kid", "k", "--sub", "x"])
            .status
            .success()
    );
}
