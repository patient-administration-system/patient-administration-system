//! Issues the PASETO v4.public tokens the API verifies (DTAC-4).
//!
//! This is the **issuing half** that pairs with [`crate::api::rest::user_auth`]: it holds an Ed25519
//! private key, signs short-lived tokens with the claim set `authentication-verifier` expects, and
//! publishes the public key set the API fetches from `PAS_PASETO_KEYS_URL`. The `pas-token-issuer`
//! binary is a thin command line over these functions.
//!
//! It issues tokens for **services** (the UI's backend-for-frontend) and for operators and test
//! tooling. It does not authenticate end users: staff and patients sign in to Keycloak, and the UI's
//! server then acts for them with `X-On-Behalf-Of`. A deployment that wants per-person tokens at
//! the API swaps this for a full authentication service; the API does not change, because it only
//! reads the key set.
//!
//! The private key never leaves the key file. Rotating means generating a new key under a new `kid`,
//! publishing both public keys until the old tokens have expired, then dropping the old one.

use std::io::{Read, Write};
use std::path::Path;

use authentication_verifier::Claims;
use base64::Engine;
use ed25519_dalek::SigningKey;
use rusty_paseto::core::{Footer, Key, Paseto, PasetoAsymmetricPrivateKey, Payload, Public, V4};

pub const DEFAULT_ISSUER: &str = "authentication-service";
pub const DEFAULT_AUDIENCE: &str = "patient-administration-system";
/// Five minutes: short enough that a leaked token is nearly useless, long enough to ride out clock skew.
pub const DEFAULT_TTL_SECS: i64 = 300;

/// What to put in a token.
#[derive(Debug, Clone)]
pub struct TokenSpec {
    pub kid: String,
    pub sub: String,
    pub name: String,
    pub email: String,
    pub issuer: String,
    pub audience: String,
    pub ttl_secs: i64,
    pub attrs: Vec<(String, String)>,
}

impl TokenSpec {
    pub fn service(kid: &str, sub: &str) -> Self {
        Self {
            kid: kid.into(),
            sub: sub.into(),
            name: sub.into(),
            email: String::new(),
            issuer: DEFAULT_ISSUER.into(),
            audience: DEFAULT_AUDIENCE.into(),
            ttl_secs: DEFAULT_TTL_SECS,
            attrs: vec![("svc".into(), "true".into())],
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 32 random bytes from the operating system.
fn random_seed() -> Result<[u8; 32], String> {
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut seed))
        .map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    Ok(seed)
}

/// Generate a key. Returns the private key as hex (the file's content) and the public key set.
pub fn generate(kid: &str) -> Result<(String, serde_json::Value), String> {
    let seed = random_seed()?;
    let key = SigningKey::from_bytes(&seed);
    Ok((hex(&seed), public_keys(&[(kid, &key)])))
}

/// The JWKS-style document the API fetches. More than one entry is how a rotation overlaps.
pub fn public_keys(keys: &[(&str, &SigningKey)]) -> serde_json::Value {
    let list: Vec<_> = keys
        .iter()
        .map(|(kid, k)| {
            let x = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(k.verifying_key().to_bytes());
            serde_json::json!({ "kty": "OKP", "crv": "Ed25519", "use": "sig", "kid": kid, "x": x })
        })
        .collect();
    serde_json::json!({ "keys": list })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Result<[u8; 32], String> {
    let s = s.trim();
    if s.len() != 64 || !s.is_ascii() {
        return Err("a key file holds 64 hex characters (a 32-byte seed)".into());
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
            .map_err(|_| "key file is not hexadecimal".to_string())?;
    }
    Ok(out)
}

/// Read a private key file.
pub fn load_key(path: &Path) -> Result<SigningKey, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read key file {}: {e}", path.display()))?;
    Ok(SigningKey::from_bytes(&unhex(&text)?))
}

/// Write a private key file readable by its owner only.
pub fn write_private(path: &Path, hex_seed: &str) -> Result<(), String> {
    write_file(path, hex_seed.as_bytes(), 0o600)
}

/// Write a file atomically (so a reader never sees half a token) with the given Unix mode.
pub fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = opts
        .open(&tmp)
        .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    f.write_all(bytes).map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}

/// Sign a token.
pub fn mint(key: &SigningKey, spec: &TokenSpec) -> Result<String, String> {
    if spec.sub.trim().is_empty() {
        return Err("a token needs a subject".into());
    }
    if !(1..=3600).contains(&spec.ttl_secs) {
        return Err(
            "ttl must be between 1 second and 1 hour: tokens are meant to be short-lived".into(),
        );
    }
    let mut attrs = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (k, v) in &spec.attrs {
        attrs.entry(k.clone()).or_default().push(v.clone());
    }
    let t = now();
    let claims = Claims {
        sub: spec.sub.clone(),
        email: spec.email.clone(),
        name: spec.name.clone(),
        iss: spec.issuer.clone(),
        aud: spec.audience.clone(),
        exp: t + spec.ttl_secs,
        iat: t,
        nbf: Some(t - 5),
        sid: uuid::Uuid::new_v4().to_string(),
        scope: vec![],
        roles: vec![],
        attrs,
    };
    let payload = serde_json::to_string(&claims).map_err(|e| e.to_string())?;
    let footer = serde_json::json!({ "kid": spec.kid }).to_string();
    let kp = Key::<64>::from(key.to_keypair_bytes());
    let private = PasetoAsymmetricPrivateKey::<V4, Public>::from(&kp);
    let mut b = Paseto::<V4, Public>::builder();
    b.set_payload(Payload::from(payload.as_str()));
    b.set_footer(Footer::from(footer.as_str()));
    b.try_sign(&private)
        .map_err(|e| format!("signing failed: {e}"))
}
