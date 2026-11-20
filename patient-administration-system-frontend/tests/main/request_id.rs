//! AXA-6: a call to the PAS API or `person-service` carries the id of the request it serves.
//!
//! Needs no database: the pool is lazy, and every call under test leaves before the first read. One
//! test, because the service URLs are read from the process environment.

use std::sync::{Arc, Mutex};

use axa::{AppContext, Environment, Result};
use axa_test::prelude::*;
use axum::{Router, middleware::from_fn, routing::get};
use patient_administration_system_frontend::app::App as Service;
use patient_administration_system_frontend::models::patient::names_for;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

/// What a peer service was sent: each call's method and path, and its `X-Request-Id`.
type Calls = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// A peer service that writes down what it is asked and answers every call with an empty success.
async fn peer() -> (String, Calls) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the peer");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    let calls = Calls::default();
    let seen = Arc::clone(&calls);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..read]).to_string();
                let mut lines = head.lines();
                let first = lines.next().unwrap_or_default();
                let call = first.rsplit_once(' ').map_or(first, |(call, _)| call);
                let id = lines.find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("x-request-id")
                        .then(|| value.trim().to_string())
                });
                seen.lock().expect("calls").push((call.to_string(), id));
                let body = r#"{"success":true,"data":[]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, calls)
}

/// The service's context with its database stopped.
fn context() -> Result<AppContext> {
    let environment = Environment::Test;
    let config = Service::config(&environment)?;
    let db = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgres://pas:pas@127.0.0.1:1/pas")
        .expect("a lazy pool does not connect");
    Ok(AppContext::new(environment, db, config))
}

/// The whole service.
struct App;

impl TestApp for App {
    async fn boot_test() -> Result<BootResult> {
        let ctx = context()?;
        let router = Service::router(&ctx)?;
        Ok(BootResult {
            app_context: ctx,
            router: Some(router),
        })
    }
}

/// `names_for` alone, tracked as `Service::router` tracks: its pages read the database first.
struct Names;

impl TestApp for Names {
    async fn boot_test() -> Result<BootResult> {
        async fn names() -> &'static str {
            names_for(&[Uuid::new_v4(), Uuid::new_v4()]).await;
            "done"
        }
        Ok(BootResult {
            app_context: context()?,
            router: Some(
                Router::new()
                    .route("/names", get(names))
                    .layer(from_fn(axa::tracing::track)),
            ),
        })
    }
}

/// Takes what the peer has been sent since the last look.
fn taken(calls: &Calls) -> Vec<(String, Option<String>)> {
    std::mem::take(&mut *calls.lock().expect("calls"))
}

#[tokio::test]
async fn every_call_to_a_peer_carries_the_id_of_its_request() {
    let (url, calls) = peer().await;
    // SAFETY: the only test in this binary, and nothing else runs yet.
    unsafe {
        std::env::set_var("PERSON_SERVICE_URL", &url);
        std::env::set_var("PAS_API_URL", &url);
    }

    let person = Uuid::new_v4();

    request::<App, _, _>(|request, _ctx| {
        let calls = Arc::clone(&calls);
        async move {
            let thing = Uuid::new_v4().to_string();
            let form = [
                ("person_id", thing.as_str()),
                ("bed_id", thing.as_str()),
                ("slot_id", thing.as_str()),
                ("template_id", thing.as_str()),
                ("channel", "post"),
                ("csrf_token", "the-token"),
            ];

            // Reads from person-service, with the id the caller sent.
            request
                .get("/patients")
                .add_header("x-request-id", "the-list")
                .await;
            let seen = taken(&calls);
            assert_eq!(seen.len(), 1, "{seen:?}");
            assert!(seen[0].0.starts_with("GET /api/persons"), "{seen:?}");
            assert_eq!(seen[0].1.as_deref(), Some("the-list"));

            request
                .get(&format!("/patients/{person}"))
                .add_header("x-request-id", "the-person")
                .await;
            assert_eq!(
                taken(&calls),
                [(
                    format!("GET /api/persons/{person}"),
                    Some("the-person".to_string())
                )]
            );

            // With no id from the caller, the peer gets the one the response names.
            let response = request.get("/patients").await;
            let made = response.header("x-request-id");
            let seen = taken(&calls);
            assert_eq!(seen[0].1.as_deref(), Some(made.to_str().unwrap()));

            // An id that is not safe is cleaned before it travels, as axa cleans it for its logs.
            request
                .get("/patients")
                .add_header("x-request-id", "a<b>c")
                .await;
            assert_eq!(taken(&calls)[0].1.as_deref(), Some("abc"));

            // Writes to the PAS API. Each answers an error here, for want of a database; the call
            // has been made by then. A person-service read follows, and carries the id too.
            for (path, id, expected) in [
                ("/admissions/new", "the-admission", "POST /api/admissions"),
                (
                    "/appointments/new",
                    "the-booking",
                    &format!("POST /api/slots/{thing}/book")[..],
                ),
                ("/letters/new", "the-letter", "POST /api/letters/generate"),
            ] {
                request
                    .post(path)
                    .add_header("x-request-id", id)
                    .add_header("cookie", "pas_csrf=the-token")
                    .form(&form)
                    .await;
                let seen = taken(&calls);
                assert_eq!(seen[0].0, expected, "{seen:?}");
                assert!(
                    seen.iter().all(|(_, sent)| sent.as_deref() == Some(id)),
                    "{seen:?}"
                );
            }
        }
    })
    .await;

    // A spawned lookup keeps the id: `tokio::spawn` would lose it.
    request::<Names, _, _>(|request, _ctx| {
        let calls = Arc::clone(&calls);
        async move {
            request
                .get("/names")
                .add_header("x-request-id", "the-names")
                .await
                .assert_status_ok();
            let seen = taken(&calls);
            assert_eq!(seen.len(), 2, "{seen:?}");
            assert!(
                seen.iter()
                    .all(|(_, sent)| sent.as_deref() == Some("the-names")),
                "{seen:?}"
            );
        }
    })
    .await;
}
