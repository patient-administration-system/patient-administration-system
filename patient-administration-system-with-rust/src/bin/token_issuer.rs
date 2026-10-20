//! `pas-token-issuer`: key generation, token minting, and a small server that publishes the public
//! keys and keeps a service-token file fresh for the UI (DTAC-4). See `token_issuer` for the design.
//!
//! ```text
//! pas-token-issuer keygen --dir DIR [--kid KID]
//! pas-token-issuer mint   --key FILE --kid KID --sub SUBJECT [--attr k=v]... [--ttl SECS] [--name N] [--email E]
//! pas-token-issuer serve  --key FILE --kid KID --sub SUBJECT --token-file FILE [--bind ADDR] [--ttl SECS] [--attr k=v]...
//! ```
//!
//! `--iss` and `--aud` default to the API's defaults. `serve` publishes `GET /keys.json` (default bind
//! 127.0.0.1:8195) and rewrites the token file every half-TTL, so `PAS_API_TOKEN_FILE` always holds a
//! live token.

#![cfg_attr(not(test), forbid(unsafe_code))]

use std::path::PathBuf;
use std::time::Duration;

use patient_administration_system::token_issuer::{self as ti, TokenSpec};

fn main() {
    if let Err(e) = run() {
        eprintln!("pas-token-issuer: {e}");
        std::process::exit(2);
    }
}

struct Args(Vec<String>);

impl Args {
    fn one(&self, name: &str) -> Option<String> {
        self.0
            .windows(2)
            .find(|w| w[0] == name)
            .map(|w| w[1].clone())
    }
    fn need(&self, name: &str) -> Result<String, String> {
        self.one(name).ok_or_else(|| format!("{name} is required"))
    }
    fn many(&self, name: &str) -> Vec<String> {
        self.0
            .windows(2)
            .filter(|w| w[0] == name)
            .map(|w| w[1].clone())
            .collect()
    }
    fn spec(&self) -> Result<TokenSpec, String> {
        let mut spec = TokenSpec::service(&self.need("--kid")?, &self.need("--sub")?);
        let attrs = self.many("--attr");
        if !attrs.is_empty() {
            spec.attrs = attrs
                .iter()
                .map(|a| {
                    a.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .ok_or_else(|| format!("--attr wants key=value, got {a}"))
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(v) = self.one("--ttl") {
            spec.ttl_secs = v
                .parse()
                .map_err(|_| "--ttl is whole seconds".to_string())?;
        }
        if let Some(v) = self.one("--iss") {
            spec.issuer = v;
        }
        if let Some(v) = self.one("--aud") {
            spec.audience = v;
        }
        if let Some(v) = self.one("--name") {
            spec.name = v;
        }
        if let Some(v) = self.one("--email") {
            spec.email = v;
        }
        Ok(spec)
    }
}

fn run() -> Result<(), String> {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        return Err(
            "usage: pas-token-issuer keygen|mint|serve (see the header of src/bin/token_issuer.rs)"
                .into(),
        );
    }
    let cmd = argv.remove(0);
    let args = Args(argv);
    match cmd.as_str() {
        "keygen" => {
            let dir = PathBuf::from(args.need("--dir")?);
            let kid = args.one("--kid").unwrap_or_else(|| "key-1".into());
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let (seed, keys) = ti::generate(&kid)?;
            ti::write_private(&dir.join("private.key"), &seed)?;
            ti::write_file(
                &dir.join("keys.json"),
                serde_json::to_string_pretty(&keys).unwrap().as_bytes(),
                0o644,
            )?;
            println!(
                "wrote {0}/private.key (keep secret) and {0}/keys.json (publish)",
                dir.display()
            );
            Ok(())
        }
        "mint" => {
            let key = ti::load_key(&PathBuf::from(args.need("--key")?))?;
            println!("{}", ti::mint(&key, &args.spec()?)?);
            Ok(())
        }
        "serve" => serve(&args),
        other => Err(format!("unknown command {other}")),
    }
}

fn serve(args: &Args) -> Result<(), String> {
    let key_path = PathBuf::from(args.need("--key")?);
    let token_file = PathBuf::from(args.need("--token-file")?);
    let spec = args.spec()?;
    let bind = args
        .one("--bind")
        .unwrap_or_else(|| "127.0.0.1:8195".into());
    let key = ti::load_key(&key_path)?;
    let keys_doc = serde_json::to_string(&ti::public_keys(&[(spec.kid.as_str(), &key)])).unwrap();
    let every = Duration::from_secs((spec.ttl_secs.max(2) / 2) as u64);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async move {
        // Mint once up front so the file exists before anything starts.
        ti::write_file(&token_file, ti::mint(&key, &spec)?.as_bytes(), 0o600)?;
        let tf = token_file.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                match ti::mint(&key, &spec).and_then(|t| ti::write_file(&tf, t.as_bytes(), 0o600)) {
                    Ok(()) => {}
                    Err(e) => eprintln!("pas-token-issuer: could not refresh the token: {e}"),
                }
            }
        });
        let app = axum::Router::new().route(
            "/keys.json",
            axum::routing::get(move || {
                let doc = keys_doc.clone();
                async move {
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        doc,
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .map_err(|e| format!("cannot bind {bind}: {e}"))?;
        eprintln!(
            "pas-token-issuer: keys at http://{bind}/keys.json; token file {} refreshed every {}s",
            token_file.display(),
            every.as_secs()
        );
        axum::serve(listener, app).await.map_err(|e| e.to_string())
    })
}
