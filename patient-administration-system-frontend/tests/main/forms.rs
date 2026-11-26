//! The three write forms and the command line, against the shared database and a peer that stands in for the
//! PAS API and `person-service`. The peer answers by the person in the form: `...1` is refused (422),
//! `...2` succeeds, `...3` answers an envelope without data.
//!
//! `#[ignore]`d like the other database tests: run with `DB_URL=… cargo test -- --ignored`.

use axa::{AppContext, Environment, Result, cli::Cli};
use axa_test::prelude::*;
use clap::Parser;
use patient_administration_system_frontend::app::App as Service;
use serial_test::serial;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const REFUSED: &str = "00000000-0000-0000-0000-000000000001";
const SUCCEEDS: &str = "00000000-0000-0000-0000-000000000002";
const NO_DATA: &str = "00000000-0000-0000-0000-000000000003";
const THING: &str = "00000000-0000-0000-0000-0000000000aa";

fn answer(request: &str) -> (u16, String) {
    let path = request.lines().next().unwrap_or_default();
    if request.contains(REFUSED) {
        return (
            422,
            r#"{"type":"about:blank","title":"Unprocessable","status":422,"detail":"that was refused"}"#
                .to_string(),
        );
    }
    if request.contains(NO_DATA) {
        return (200, r#"{"success":true}"#.to_string());
    }
    let id = "00000000-0000-0000-0000-0000000000bb";
    let data = if path.contains("/api/admissions") {
        format!(
            r#"{{"admission":{{"id":"{id}","bed_id":"{id}","encounter_id":"{id}"}},"encounter":{{"id":"{id}","person_id":"{SUCCEEDS}"}}}}"#
        )
    } else if path.contains("/book") {
        format!(
            r#"{{"id":"{id}","person_id":"{SUCCEEDS}","slot_id":null,"start_datetime":"2030-01-01T09:00:00Z","end_datetime":"2030-01-01T09:30:00Z","status":"booked"}}"#
        )
    } else if path.contains("/api/letters/generate") {
        format!(
            r#"{{"id":"{id}","person_id":"{SUCCEEDS}","template_id":"{id}","rendered_subject":"Hello","rendered_body":"Body","channel":"post","status":"pending"}}"#
        )
    } else if path.starts_with("GET /api/persons/") {
        format!(
            r#"{{"id":"{SUCCEEDS}","active":true,"name":[{{"family":"Lovelace","given":["Ada"]}}],"gender":"female","birth_date":"1990-01-15","identifiers":[{{"value":"MRN-1"}}],"telecom":[{{"system":"phone","value":"0100"}}],"addresses":[{{"line":["1 High Street"]}}]}}"#
        )
    } else {
        "[]".to_string()
    };
    (200, format!(r#"{{"success":true,"data":{data}}}"#))
}

async fn peer() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the peer");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 16384];
                let mut read = socket.read(&mut buffer).await.unwrap_or(0);
                // The body can arrive after the head.
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                if let Ok(more) = tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    socket.read(&mut buffer[read..]),
                )
                .await
                {
                    read += more.unwrap_or(0);
                }
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let (status, body) = answer(&request);
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    url
}

struct App;

impl TestApp for App {
    async fn boot_test() -> Result<BootResult> {
        let environment = Environment::Test;
        let config = Service::config(&environment)?;
        let ctx: AppContext = Service::context(environment, config).await?;
        let router = Service::router(&ctx)?;
        Ok(BootResult {
            app_context: ctx,
            router: Some(router),
        })
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn each_form_shows_what_the_pas_answered() {
    let url = peer().await;
    // SAFETY: serialized with the other tests that set it, and set before the service reads it.
    unsafe {
        std::env::set_var("PERSON_SERVICE_URL", &url);
        std::env::set_var("PAS_API_URL", &url);
    }
    request::<App, _, _>(|request, _ctx| async move {
        for (path, extra) in [
            ("/admissions/new", ("bed_id", THING)),
            ("/appointments/new", ("slot_id", THING)),
            ("/letters/new", ("template_id", THING)),
        ] {
            for person in [REFUSED, SUCCEEDS, NO_DATA] {
                let response = request
                    .post(path)
                    .add_header("cookie", "pas_csrf=the-token")
                    .form(&[
                        ("person_id", person),
                        extra,
                        ("channel", "post"),
                        ("csrf_token", "the-token"),
                    ])
                    .await;
                response.assert_status_ok();
                let body = response.text();
                match person {
                    REFUSED => assert!(body.contains("that was refused"), "{path}: {body}"),
                    NO_DATA => assert!(body.contains("malformed"), "{path}: {body}"),
                    _ => assert!(!body.contains("PAS API"), "{path}: {body}"),
                }
            }
        }
        let response = request.get("/appointments/new").await;
        response.assert_status_ok();
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn the_command_line_checks_reports_and_refuses_what_it_must() {
    let run = |arguments: &[&str]| {
        let cli = Cli::parse_from(std::iter::once("frontend").chain(arguments.iter().copied()));
        async move { Service::run(cli).await }
    };
    run(&["--environment", "test", "check"]).await.unwrap();
    run(&["--environment", "test", "db", "status"])
        .await
        .unwrap();
    run(&["--environment", "test", "db", "migrate"])
        .await
        .unwrap();
    run(&["--environment", "test", "task"]).await.unwrap();
    assert!(
        run(&["--environment", "test", "task", "nothing"])
            .await
            .is_err()
    );
    assert!(
        run(&["--environment", "test", "db", "reset"])
            .await
            .is_err()
    );
    assert!(
        run(&["--environment", "test", "db", "truncate"])
            .await
            .is_err()
    );
}

/// A ward with a bed in each state, a pathway past its breach, and a free slot, written straight to the
/// database: the pages render rows only when there are some.
async fn some_of_everything(db: &sqlx::PgPool) -> (Uuid, String) {
    let tag = Uuid::new_v4().simple().to_string()[..10].to_string();
    let (facility, ward, room, schedule) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    sqlx::query("INSERT INTO facilities (id, name, code, address, active, created_at, updated_at) VALUES ($1, $2, $2, '{}'::jsonb, true, now(), now())")
        .bind(facility).bind(format!("F{tag}")).execute(db).await.unwrap();
    sqlx::query("INSERT INTO wards (id, facility_id, name, code, capacity, active, created_at, updated_at) VALUES ($1, $2, $3, $3, 3, true, now(), now())")
        .bind(ward).bind(facility).bind(format!("W{tag}")).execute(db).await.unwrap();
    sqlx::query("INSERT INTO rooms (id, ward_id, name, code, active, created_at, updated_at) VALUES ($1, $2, 'r', $3, true, now(), now())")
        .bind(room).bind(ward).bind(format!("R{tag}")).execute(db).await.unwrap();
    for status in ["available", "occupied", "cleaning"] {
        sqlx::query("INSERT INTO beds (id, room_id, name, code, status, created_at, updated_at) VALUES ($1, $2, 'b', $3, $4, now(), now())")
            .bind(Uuid::new_v4()).bind(room).bind(format!("B{status}{tag}")).bind(status).execute(db).await.unwrap();
    }
    let pathway = Uuid::new_v4();
    sqlx::query("INSERT INTO rtt_pathways (id, person_id, target_service, breach_weeks, status, started_at, created_at, updated_at) VALUES ($1, $2, $3, 18, 'active', now() - interval '30 weeks', now(), now())")
        .bind(pathway).bind(Uuid::parse_str(SUCCEEDS).unwrap()).bind(format!("svc-{tag}")).execute(db).await.unwrap();
    for (kind, weeks_ago) in [("started", 30), ("paused", 20), ("resumed", 19)] {
        sqlx::query("INSERT INTO rtt_clock_events (id, pathway_id, kind, event_at, created_at) VALUES ($1, $2, $3, now() - make_interval(weeks => $4), now())")
            .bind(Uuid::new_v4()).bind(pathway).bind(kind).bind(weeks_ago).execute(db).await.unwrap();
    }
    sqlx::query("INSERT INTO schedules (id, owner_kind, owner_id, service_type, active, created_at, updated_at) VALUES ($1, 'practitioner', $2, $3, true, now(), now())")
        .bind(schedule).bind(Uuid::new_v4()).bind(format!("svc-{tag}")).execute(db).await.unwrap();
    sqlx::query("INSERT INTO slots (id, schedule_id, start_datetime, end_datetime, status, created_at, updated_at) VALUES ($1, $2, now() + interval '1 day', now() + interval '1 day 30 minutes', 'free', now(), now())")
        .bind(Uuid::new_v4()).bind(schedule).execute(db).await.unwrap();
    (ward, tag)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn the_pages_render_the_rows_the_database_holds() {
    let url = peer().await;
    // SAFETY: serialized with the other tests that set it, and set before the service reads it.
    unsafe {
        std::env::set_var("PERSON_SERVICE_URL", &url);
        std::env::set_var("PAS_API_URL", &url);
    }
    request::<App, _, _>(|request, ctx| async move {
        let (ward, tag) = some_of_everything(&ctx.db).await;
        for path in [
            "/dashboard",
            "/dashboard/wards",
            "/dashboard/outbox",
            "/dashboard/audit",
            "/rtt",
            "/appointments/new",
            "/admissions/new",
            "/letters/new",
            &format!("/wards/{ward}"),
            &format!("/patients/{SUCCEEDS}"),
        ] {
            let response = request.get(path).await;
            response.assert_status_ok();
        }
        let wards = request.get("/dashboard/wards").await.text();
        assert!(wards.contains(&format!("W{tag}")), "{wards}");
        let rtt = request.get("/rtt").await.text();
        assert!(rtt.contains(&format!("svc-{tag}")), "{rtt}");
    })
    .await;
}
