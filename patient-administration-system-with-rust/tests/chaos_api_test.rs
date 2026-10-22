//! Chaos testing, part 1: a seeded monkey hits every route with hostile input.
//!
//! It reads the route table from `src/api/rest/routes/`, so a new route is covered the day it is added. For
//! each request it picks a method that route accepts, fills the path parameters with a mix of plausible and
//! hostile values (random ids, `../`, NUL, very long strings, unicode), adds a junk query string, and sends a
//! generated JSON body (or no body, malformed JSON, or the wrong content type). Whatever it sends, the API must:
//!
//! - never panic (a panic would abort the request task);
//! - never answer 500 (a 5xx is allowed only as 502/503/504 for an upstream that is not configured);
//! - never put a stack trace, SQL or a file path into a response.
//!
//! It is seeded: a failure prints `PAS_CHAOS_SEED` and the request that broke, and the same seed repeats it.
//! `PAS_CHAOS_ITERATIONS` (default 2000) sets the length. Needs PostgreSQL; it refuses to run except on a database whose name contains "chaos", in CI, or with `PAS_CHAOS_ANY_DB=1`; run it on a throwaway database
//! (`cargo test --test chaos_api_test -- --ignored`), because it writes whatever the API lets it write.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

/// xorshift64*: small, seedable, and the same on every platform.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

struct Route {
    path: String,
    methods: Vec<&'static str>,
}

/// Every `.route("path", get(..).post(..))` in the table, with the methods it registers.
fn routes() -> Vec<Route> {
    // The route table is split by area: every file under `src/api/rest/routes/`.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/rest/routes");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 6,
        "expected the whole route table, found {files:?}"
    );
    let src: String = files
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let src = src.as_str();
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find(".route(") {
        rest = &rest[i + 7..];
        let end = rest.find(".route(").unwrap_or(rest.len());
        let stmt = &rest[..end];
        let Some(q1) = stmt.find('"') else { continue };
        let Some(q2) = stmt[q1 + 1..].find('"') else {
            continue;
        };
        let path = stmt[q1 + 1..q1 + 1 + q2].to_string();
        if !path.starts_with('/') {
            continue;
        }
        let tail = &stmt[q1 + 2 + q2..];
        let mut methods = Vec::new();
        for (name, m) in [
            ("get(", "GET"),
            ("post(", "POST"),
            ("put(", "PUT"),
            ("delete(", "DELETE"),
            ("patch(", "PATCH"),
        ] {
            if tail.contains(name) {
                methods.push(m);
            }
        }
        if methods.is_empty() {
            methods.push("GET");
        }
        out.push(Route { path, methods });
    }
    out
}

fn junk_string(r: &mut Rng) -> String {
    let long = "A".repeat(2000 + r.below(5000));
    let uuid = format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        r.next() as u32,
        r.next() as u16,
        r.next() as u16 & 0xfff,
        r.next() as u16 & 0xfff,
        r.next() & 0xffff_ffff_ffff
    );
    r.pick(&[
        uuid.clone(),
        uuid,
        "not-a-uuid".into(),
        "../../etc/passwd".into(),
        "%00".into(),
        "0".into(),
        "-1".into(),
        "9999999999999999999999".into(),
        "💥🔥ñ\u{202e}rtl".into(),
        "' OR '1'='1".into(),
        "\"; DROP TABLE appointments; --".into(),
        "<script>alert(1)</script>".into(),
        "{{7*7}}".into(),
        "${jndi:ldap://x}".into(),
        long,
        "null".into(),
        "true".into(),
        " ".into(),
    ])
    .clone()
}

fn junk_value(r: &mut Rng, depth: u32) -> Value {
    match r.below(if depth > 2 { 7 } else { 10 }) {
        0 => Value::Null,
        1 => json!(r.chance(50)),
        2 => json!(r.next() as i64),
        3 => json!(-(r.next() as i64 % 1000)),
        4 => json!(f64::from_bits(r.next()).clamp(-1e300, 1e300)),
        5 | 6 => Value::String(junk_string(r)),
        7 => Value::Array((0..r.below(4)).map(|_| junk_value(r, depth + 1)).collect()),
        _ => junk_object(r, depth + 1),
    }
}

/// Field names the real handlers use, so the monkey gets past the first validation line and into the logic.
const KEYS: &[&str] = &[
    "person_id",
    "appointment_id",
    "slot_id",
    "schedule_id",
    "id",
    "name",
    "status",
    "action",
    "message_id",
    "channel",
    "from",
    "body",
    "token",
    "digit",
    "state",
    "reason",
    "reason_code",
    "reviewer",
    "outcome",
    "notes",
    "kind",
    "stage",
    "changed_by",
    "note",
    "asked_by",
    "recorded_by",
    "approved_by",
    "source",
    "age_band",
    "deprivation_quintile",
    "mobile",
    "email",
    "preferred_language",
    "start_datetime",
    "end_datetime",
    "service",
    "service_type",
    "priority",
    "limit",
    "k",
    "weeks",
    "template_id",
    "subject",
    "body_tera",
    "channels",
    "active",
];

fn junk_object(r: &mut Rng, depth: u32) -> Value {
    let mut m = serde_json::Map::new();
    for _ in 0..r.below(8) {
        let key = if r.chance(80) {
            (*r.pick(KEYS)).to_string()
        } else {
            junk_string(r)
        };
        m.insert(key, junk_value(r, depth));
    }
    Value::Object(m)
}

struct Req {
    method: &'static str,
    uri: String,
    content_type: Option<&'static str>,
    body: Vec<u8>,
}

fn build(r: &mut Rng, routes: &[Route]) -> Req {
    let route = r.pick(routes);
    let method = *r.pick(&route.methods);
    // Fill each `{param}` with something; a handful of values contain `/` or `?`, which must not escape the route.
    let mut uri = String::new();
    let mut chars = route.path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            for d in chars.by_ref() {
                if d == '}' {
                    break;
                }
            }
            let v = junk_string(r);
            uri.push_str(&urlencode(&v));
        } else {
            uri.push(c);
        }
    }
    if r.chance(60) {
        uri.push('?');
        for i in 0..r.below(4) {
            if i > 0 {
                uri.push('&');
            }
            let key = if r.chance(70) {
                (*r.pick(KEYS)).to_string()
            } else {
                urlencode(&junk_string(r))
            };
            uri.push_str(&format!("{key}={}", urlencode(&junk_string(r))));
        }
    }
    let (content_type, body) = match r.below(10) {
        0 => (None, Vec::new()),
        1 => (
            Some("application/json"),
            b"{\"unterminated\": [1, 2".to_vec(),
        ),
        2 => (Some("text/plain"), junk_string(r).into_bytes()),
        3 => (Some("application/json"), b"\xff\xfe\x00garbage".to_vec()),
        4 => (
            Some("application/json"),
            serde_json::to_vec(&junk_value(r, 0)).unwrap(),
        ),
        _ => (
            Some("application/json"),
            serde_json::to_vec(&junk_object(r, 0)).unwrap(),
        ),
    };
    Req {
        method,
        uri,
        content_type,
        body,
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// What a response must never contain, whatever went wrong inside.
const LEAKS: &[&str] = &[
    "panicked at",
    "stack backtrace",
    "RUST_BACKTRACE",
    "/Users/",
    "/home/",
    "src/",
    ".rs:",
    "syntax error at or near",
    "relation \"",
    "duplicate key value",
    "violates",
    "SqlxError",
    "PgDatabaseError",
    "DbErr",
];

async fn send(app: &Router, req: &Req) -> Result<(StatusCode, String), String> {
    let mut b = Request::builder().method(req.method).uri(&req.uri);
    if let Some(ct) = req.content_type {
        b = b.header("content-type", ct);
    }
    let request = match b.body(Body::from(req.body.clone())) {
        Ok(r) => r,
        // The monkey built something that is not even a valid HTTP request line: nothing for the API to do.
        Err(_) => return Ok((StatusCode::BAD_REQUEST, String::new())),
    };
    let app = app.clone();
    let handle = tokio::spawn(async move {
        let resp = app.oneshot(request).await.unwrap();
        let st = resp.status();
        let bytes = to_bytes(resp.into_body(), 16 << 20)
            .await
            .unwrap_or_default();
        (st, String::from_utf8_lossy(&bytes).to_string())
    });
    handle
        .await
        .map_err(|e| format!("the request task panicked: {e}"))
}

fn seed() -> u64 {
    std::env::var("PAS_CHAOS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1)
                | 1
        })
}

#[tokio::test]
#[ignore = "chaos: requires PostgreSQL (DATABASE_URL), writes junk; run on a throwaway database"]
async fn no_route_panics_answers_500_or_leaks_internals_whatever_it_is_sent() {
    // It writes junk. Run it only on a database made for it (a name containing "chaos"), in CI (a database made
    // for the run), or when told it may (`PAS_CHAOS_ANY_DB=1`).
    let url = common::database_url().unwrap();
    if !url.contains("chaos")
        && std::env::var("CI").is_err()
        && std::env::var("PAS_CHAOS_ANY_DB").is_err()
    {
        eprintln!(
            "skipped: DATABASE_URL is not a chaos database (name it ...pas_chaos, or set PAS_CHAOS_ANY_DB=1)"
        );
        return;
    }
    let seed = seed();
    let iterations: usize = std::env::var("PAS_CHAOS_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let routes = routes();
    assert!(
        routes.len() > 150,
        "the route table was not read: {} routes",
        routes.len()
    );
    let s = common::build_state(&url).await;
    let app = patient_administration_system::api::rest::router(s);
    let mut r = Rng(seed);
    let mut by_status = std::collections::BTreeMap::<u16, usize>::new();
    let mut failures = Vec::new();
    for i in 0..iterations {
        let req = build(&mut r, &routes);
        let shown = format!(
            "#{i} {} {} [{}] {}",
            req.method,
            req.uri.chars().take(200).collect::<String>(),
            req.content_type.unwrap_or("-"),
            String::from_utf8_lossy(&req.body)
                .chars()
                .take(300)
                .collect::<String>()
        );
        match send(&app, &req).await {
            Err(e) => failures.push(format!("{shown}\n   -> {e}")),
            Ok((st, body)) => {
                *by_status.entry(st.as_u16()).or_default() += 1;
                if st.is_server_error() && ![502, 503, 504].contains(&st.as_u16()) {
                    failures.push(format!(
                        "{shown}\n   -> {st}: {}",
                        body.chars().take(300).collect::<String>()
                    ));
                }
                if let Some(leak) = LEAKS.iter().find(|l| body.contains(*l)) {
                    failures.push(format!(
                        "{shown}\n   -> {st} leaks `{leak}`: {}",
                        body.chars().take(300).collect::<String>()
                    ));
                }
            }
        }
        if failures.len() >= 15 {
            break;
        }
    }
    eprintln!("chaos seed {seed}, {iterations} requests, statuses {by_status:?}");
    assert!(
        failures.is_empty(),
        "PAS_CHAOS_SEED={seed}\n{}",
        failures.join("\n")
    );
}

#[test]
fn test_the_monkey_reads_the_whole_route_table_and_is_repeatable() {
    let routes = routes();
    assert!(routes.len() > 150, "{} routes", routes.len());
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/api/noshow/rollout/{service_type}" && r.methods.contains(&"PUT"))
    );
    assert!(routes.iter().all(|r| !r.methods.is_empty()));
    let (mut a, mut b) = (Rng(42), Rng(42));
    let first: Vec<String> = (0..50).map(|_| build(&mut a, &routes).uri).collect();
    let second: Vec<String> = (0..50).map(|_| build(&mut b, &routes).uri).collect();
    assert_eq!(first, second, "the same seed sends the same requests");
}
