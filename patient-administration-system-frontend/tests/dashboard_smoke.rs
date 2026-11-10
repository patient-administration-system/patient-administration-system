//! Smoke test for the front-end's HTTP surface.
//!
//! Boots the app via axa's testing harness and asserts that
//! `/dashboard`, `/patients`, and `/_health` all return 2xx + the
//! expected markup (or JSON shape). `#[ignore]`d like the PAS Axum
//! integration tests: run with `DB_URL=… cargo test -- --ignored`, as `just rust-db` does.

use axa::{Environment, Result, cli::Cli};
use axa_test::prelude::*;
use clap::Parser;
use patient_administration_system_frontend::app::App as Service;
use serial_test::serial;

/// The service as the test harness boots it. The database is the shared PAS one, already migrated
/// and never reset here.
struct App;

impl TestApp for App {
    async fn boot_test() -> Result<BootResult> {
        let environment = Environment::Test;
        let config = Service::config(&environment)?;
        let ctx = Service::context(environment, config).await?;
        let router = Service::router(&ctx)?;
        Ok(BootResult {
            app_context: ctx,
            router: Some(router),
        })
    }
}

/// These tests are `#[ignore]`d; asking for them without a database is a failure, not a skip.
fn database_url() -> Option<String> {
    Some(
        std::env::var("DB_URL")
            .expect("DB_URL is required: these tests need PostgreSQL (see podman/compose.yml)"),
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn dashboard_renders_lily_markup() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping dashboard_renders_lily_markup");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/dashboard").await;
        resp.assert_status_ok();
        let body = resp.text();

        // The policy is for pages: axa's default would blank this one.
        let policy = resp.header("content-security-policy");
        assert!(policy.to_str().unwrap().contains("style-src 'self'"));

        // HTMX is loaded as a CDN script.
        assert!(body.contains("/static/js/htmx-"), "page should load HTMX");

        // The four polling panels are wired with hx-get URLs.
        for hx in [
            "hx-get=\"/dashboard/wards\"",
            "hx-get=\"/dashboard/outbox\"",
            "hx-get=\"/dashboard/audit\"",
        ] {
            assert!(body.contains(hx), "missing {hx}");
        }

        // Lily Design System semantic class names appear.
        for class in ["class=\"header\"", "class=\"footer\"", "class=\"panel\""] {
            assert!(body.contains(class), "missing Lily class {class}");
        }

        // ARIA tightening: every panel is a role="region".
        assert!(
            body.matches("role=\"region\"").count() >= 3,
            "expected at least 3 ARIA region panels"
        );

        // Footer attributes the markup to Lily Design System.
        assert!(body.contains("Lily Design System"));
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn dashboard_fragments_return_body_only() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping dashboard_fragments_return_body_only");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        for path in ["/dashboard/wards", "/dashboard/outbox", "/dashboard/audit"] {
            let resp = request.get(path).await;
            resp.assert_status_ok();
            let body = resp.text();
            // Fragments must NOT include the page chrome (header / HTMX
            // script tag), otherwise HTMX would swap a full HTML doc
            // into the panel-body div.
            assert!(
                !body.contains("<header"),
                "fragment {path} must not include the <header>; got: {body}"
            );
            assert!(
                !body.contains("/static/js/htmx-"),
                "fragment {path} must not embed the HTMX script; got: {body}"
            );
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn patients_index_renders_lily_table() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping patients_index_renders_lily_table");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/patients").await;
        resp.assert_status_ok();
        let body = resp.text();
        // Either the empty-state Lily `alert` or the populated Lily
        // `data-table` is fine — both prove the template renders.
        let has_table = body.contains("class=\"data-table\"");
        let has_empty = body.contains("No patients yet");
        assert!(
            has_table || has_empty,
            "patients page must render either an empty alert or a data-table; got: {body}"
        );
        assert!(body.contains("class=\"panel\""));
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn admission_form_renders() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping admission_form_renders");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/admissions/new").await;
        resp.assert_status_ok();
        let body = resp.text();
        assert!(body.contains("New admission"));
        assert!(body.contains("class=\"panel\""));
        let has_form = body.contains("<form");
        let has_empty = body.contains("No available beds") || body.contains("No patients yet");
        assert!(
            has_form || has_empty,
            "admissions form must render either an empty-state alert or a form"
        );
        assert!(!body.contains("Admitted</strong>"));
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn appointment_form_renders() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping appointment_form_renders");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/appointments/new").await;
        resp.assert_status_ok();
        let body = resp.text();
        assert!(body.contains("Book appointment"));
        assert!(body.contains("class=\"panel\""));
        let has_form = body.contains("<form");
        let has_empty = body.contains("No free slots") || body.contains("No patients yet");
        assert!(
            has_form || has_empty,
            "appointments form must render either an empty-state alert or a form"
        );
        assert!(!body.contains("Booked</strong>"));
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn letter_composer_renders_form() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping letter_composer_renders_form");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/letters/new").await;
        resp.assert_status_ok();
        let body = resp.text();
        assert!(body.contains("Compose letter"));
        // Either the empty-state Lily alert (no templates) or the actual
        // <form> is fine; both prove the template + DB queries ran.
        let has_form = body.contains("<form");
        let has_empty = body.contains("No active letter templates");
        assert!(
            has_form || has_empty,
            "letters/new must render either an empty-state alert or a <form>; got: {body}"
        );
        assert!(body.contains("class=\"panel\""));
        // No PAS API call happens on GET — we should NOT see the success banner.
        assert!(!body.contains("Generated</strong>"));
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn rtt_cockpit_renders_lily_markup() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping rtt_cockpit_renders_lily_markup");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/rtt").await;
        resp.assert_status_ok();
        let body = resp.text();
        // Page chrome + Lily panel.
        assert!(body.contains("class=\"panel\""));
        assert!(body.contains("RTT cockpit"));
        // Either the populated data-table or the empty-state alert is
        // fine; either proves the template renders.
        let has_table = body.contains("class=\"data-table\"");
        let has_empty = body.contains("No active RTT pathways");
        assert!(
            has_table || has_empty,
            "RTT page must render either an empty alert or a data-table; got: {body}"
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn unknown_ward_returns_404() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping unknown_ward_returns_404");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request
            .get("/wards/00000000-0000-0000-0000-000000000000")
            .await;
        // axa maps `Error::NotFound` → 404.
        resp.assert_status_not_found();
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn admission_get_sets_csrf_cookie_and_field() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping admission_get_sets_csrf_cookie_and_field");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/admissions/new").await;
        resp.assert_status_ok();

        // GET must always set the pas_csrf cookie, even when the form
        // is in its empty-state branch.
        let cookie = resp
            .maybe_cookie(patient_administration_system_frontend::csrf::COOKIE_NAME)
            .expect("GET /admissions/new must Set-Cookie pas_csrf");
        assert!(
            !cookie.value().is_empty(),
            "pas_csrf cookie must carry a non-empty token"
        );

        // If the page is showing the actual form (not the empty-state
        // alert), it must embed the hidden csrf_token field.
        let body = resp.text();
        if body.contains("<form") {
            assert!(
                body.contains("name=\"csrf_token\""),
                "rendered form must include the hidden csrf_token field"
            );
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn admission_post_with_bad_csrf_returns_400() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping admission_post_with_bad_csrf_returns_400");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        // No prior GET → no pas_csrf cookie on the client → verify_token
        // sees an empty cookie value and rejects with BadRequest (400).
        let resp = request
            .post("/admissions/new")
            .form(&[
                ("person_id", "00000000-0000-0000-0000-000000000000"),
                ("bed_id", "00000000-0000-0000-0000-000000000000"),
                ("csrf_token", "not-the-real-token"),
            ])
            .await;
        resp.assert_status_bad_request();
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn appointment_post_with_bad_csrf_returns_400() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping appointment_post_with_bad_csrf_returns_400");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request
            .post("/appointments/new")
            .form(&[
                ("slot_id", "00000000-0000-0000-0000-000000000000"),
                ("person_id", "00000000-0000-0000-0000-000000000000"),
                ("csrf_token", "not-the-real-token"),
            ])
            .await;
        resp.assert_status_bad_request();
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn letter_post_with_bad_csrf_returns_400() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping letter_post_with_bad_csrf_returns_400");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request
            .post("/letters/new")
            .form(&[
                ("template_id", "00000000-0000-0000-0000-000000000000"),
                ("person_id", "00000000-0000-0000-0000-000000000000"),
                ("channel", "print"),
                ("csrf_token", "not-the-real-token"),
            ])
            .await;
        resp.assert_status_bad_request();
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn health_endpoint_returns_json() {
    if database_url().is_none() {
        eprintln!("DB_URL not set; skipping health_endpoint_returns_json");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/_health").await;
        resp.assert_status_ok();
        let v: serde_json::Value = resp.json();
        assert_eq!(v["service"], "patient-administration-system-frontend");
        assert_eq!(v["database"], "ok");
        assert!(
            v["version"].as_str().is_some(),
            "health response must include version"
        );
    })
    .await;
}

/// The service with its database stopped: a pool of connections to a port nothing listens on. Needs no database.
struct StoppedDatabase;

impl TestApp for StoppedDatabase {
    async fn boot_test() -> Result<BootResult> {
        let environment = Environment::Test;
        let config = Service::config(&environment)?;
        let db = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(200))
            .connect_lazy("postgres://pas:pas@127.0.0.1:1/pas")
            .expect("a lazy pool does not connect");
        let ctx = axa::AppContext::new(environment, db, config);
        let router = Service::router(&ctx)?;
        Ok(BootResult {
            app_context: ctx,
            router: Some(router),
        })
    }
}

/// The service with a database that takes connections and never answers, as a hung server or a dropped route
/// does: each query waits for the pool's one-second timeout. Needs no database.
struct HangingDatabase;

impl TestApp for HangingDatabase {
    async fn boot_test() -> Result<BootResult> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().expect("its address").port();
        // Accept every connection and hold it open, saying nothing.
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().flatten() {
                held.push(stream);
            }
        });
        let environment = Environment::Test;
        let config = Service::config(&environment)?;
        let db = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect_lazy(&format!("postgres://pas:pas@127.0.0.1:{port}/pas"))
            .expect("a lazy pool does not connect");
        let ctx = axa::AppContext::new(environment, db, config);
        let router = Service::router(&ctx)?;
        Ok(BootResult {
            app_context: ctx,
            router: Some(router),
        })
    }
}

/// A database that hangs costs the dashboard one connect timeout, not one per panel: its three panels are read
/// at once, and each says the database cannot be reached.
#[tokio::test]
async fn a_hanging_database_costs_the_dashboard_one_timeout() {
    request::<HangingDatabase, _, _>(|request, _ctx| async move {
        let started = std::time::Instant::now();
        let resp = request.get("/dashboard").await;
        let took = started.elapsed();
        resp.assert_status_ok();
        assert_eq!(
            resp.text()
                .matches("The database cannot be reached")
                .count(),
            3
        );
        assert!(
            took < std::time::Duration::from_millis(2_500),
            "three one-second timeouts one after another: {took:?}"
        );
    })
    .await;
}

/// AXA-2: what the pages load comes from this service. Needs no database.
#[tokio::test]
async fn the_page_assets_are_served_from_this_service() {
    request::<StoppedDatabase, _, _>(|request, _ctx| async move {
        for (path, kind) in [
            ("/static/css/app.css", "text/css"),
            ("/static/js/htmx-2.0.3.min.js", "javascript"),
        ] {
            let resp = request.get(path).await;
            resp.assert_status_ok();
            let content_type = resp.header("content-type");
            assert!(
                content_type.to_str().unwrap().contains(kind),
                "{path}: {content_type:?}"
            );
        }
    })
    .await;
}

/// axa 0.6: every refusal is an RFC 9457 problem, `code` and `detail` in place of `error` and `description`,
/// including a bare `404` that had no body. Needs no database.
#[tokio::test]
async fn a_refusal_is_a_problem() {
    request::<StoppedDatabase, _, _>(|request, _ctx| async move {
        let resp = request
            .post("/admissions/new")
            .form(&[
                ("person_id", "00000000-0000-0000-0000-000000000000"),
                ("bed_id", "00000000-0000-0000-0000-000000000000"),
                ("csrf_token", "not-the-real-token"),
            ])
            .await;
        resp.assert_status_bad_request();
        assert_eq!(resp.header("content-type"), "application/problem+json");
        let body: serde_json::Value = resp.json();
        assert_eq!(body["status"], 400, "{body}");
        assert_eq!(body["code"], "bad_request", "{body}");
        assert_eq!(body["detail"], "CSRF token mismatch", "{body}");
        assert!(
            body.get("error").is_none() && body.get("description").is_none(),
            "{body}"
        );

        for path in ["/no-such-page", "/static/no-such-file.css"] {
            let resp = request.get(path).await;
            resp.assert_status_not_found();
            assert_eq!(
                resp.header("content-type"),
                "application/problem+json",
                "{path}"
            );
            let body: serde_json::Value = resp.json();
            assert_eq!(body["status"], 404, "{path}: {body}");
        }

        // axa::extract::Path: a malformed id says what was wrong, and quotes nothing the caller sent.
        for path in ["/wards/secret-not-a-uuid", "/patients/secret-not-a-uuid"] {
            let resp = request.get(path).await;
            resp.assert_status_bad_request();
            let body: serde_json::Value = resp.json();
            assert_eq!(body["code"], "bad_request", "{path}: {body}");
            assert!(body["detail"].is_string(), "{path}: {body}");
            assert!(!body.to_string().contains("secret"), "{path}: {body}");
        }
    })
    .await;
}

/// With the database down each dashboard panel says so, and none says "0 unpublished", "no wards" or "no
/// audit entries", which read as all clear. Needs no database.
#[tokio::test]
async fn the_dashboard_says_when_it_cannot_reach_the_database() {
    request::<StoppedDatabase, _, _>(|request, _ctx| async move {
        for path in [
            "/dashboard",
            "/dashboard/wards",
            "/dashboard/outbox",
            "/dashboard/audit",
        ] {
            let resp = request.get(path).await;
            resp.assert_status_ok();
            let body = resp.text();
            assert!(
                body.contains("The database cannot be reached"),
                "{path}: {body}"
            );
            for all_clear in [
                "All events delivered",
                "No wards configured",
                "No audit entries",
            ] {
                assert!(
                    !body.contains(all_clear),
                    "{path} says {all_clear:?} while the database is down"
                );
            }
        }
        let page = request.get("/dashboard").await.text();
        assert_eq!(
            page.matches("The database cannot be reached").count(),
            3,
            "one per panel"
        );
    })
    .await;
}

/// AXA-4: an orchestrator is told the truth while the database is down, and the process stays alive for it.
#[tokio::test]
async fn readiness_fails_while_the_database_is_down_and_liveness_does_not() {
    request::<StoppedDatabase, _, _>(|request, _ctx| async move {
        let ready = request.get("/_health/ready").await;
        assert_eq!(ready.status_code(), 503);
        let v: serde_json::Value = ready.json();
        assert_eq!(v["ok"], false);
        assert_eq!(v["failed"][0], "database");

        request.get("/_health/live").await.assert_status_ok();
        let health = request.get("/_health").await;
        health.assert_status_ok();
        let v: serde_json::Value = health.json();
        assert_eq!(v["database"], "unreachable");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn readiness_passes_with_the_database_up() {
    database_url();
    request::<App, _, _>(|request, _ctx| async move {
        let ready = request.get("/_health/ready").await;
        ready.assert_status_ok();
        let v: serde_json::Value = ready.json();
        assert_eq!(v["ok"], true);
    })
    .await;
}

/// A one-purpose person-service: answers `GET /api/persons/{id}` with a
/// fixed name and returns its base URL.
async fn stub_person_service() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub person-service");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let id = head
                    .split_whitespace()
                    .nth(1)
                    .and_then(|p| p.rsplit('/').next())
                    .unwrap_or("")
                    .to_string();
                let body = format!(
                    r#"{{"success":true,"data":{{"id":"{id}","name":{{"family":"Stubfamily","given":["Sam"]}}}}}}"#
                );
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    url
}

/// BUG-8: the ward and RTT pages used to read a `patients` table that no
/// longer exists. With an occupied bed and an active pathway they must
/// render, with the name taken from person-service.
#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn ward_and_rtt_pages_render_with_an_occupied_bed_and_an_active_pathway() {
    use sqlx::AssertSqlSafe;
    let url = stub_person_service().await;
    // SAFETY: #[serial] keeps every other test in this binary from running now.
    unsafe { std::env::set_var("PERSON_SERVICE_URL", &url) };
    request::<App, _, _>(|request, ctx| async move {
        let ids: Vec<uuid::Uuid> = (0..8).map(|_| uuid::Uuid::new_v4()).collect();
        let [facility, ward, room, bed, encounter, assignment, person, pathway] =
            [0, 1, 2, 3, 4, 5, 6, 7].map(|i| ids[i]);
        let tag = &facility.simple().to_string()[..8];
        let seed = format!(
            "INSERT INTO facilities (id,name,code,address,created_at,updated_at) VALUES ('{facility}','F {tag}','F{tag}','{{}}',now(),now());
             INSERT INTO wards (id,facility_id,name,code,capacity,created_at,updated_at) VALUES ('{ward}','{facility}','W {tag}','W{tag}',1,now(),now());
             INSERT INTO rooms (id,ward_id,name,code,created_at,updated_at) VALUES ('{room}','{ward}','R {tag}','R{tag}',now(),now());
             INSERT INTO beds (id,room_id,name,code,created_at,updated_at) VALUES ('{bed}','{room}','B {tag}','B{tag}',now(),now());
             INSERT INTO encounters (id,person_id,class,status,period_start,created_at,updated_at) VALUES ('{encounter}','{person}','inpatient','in-progress',now(),now(),now());
             INSERT INTO bed_assignments (id,encounter_id,bed_id,assigned_at,created_at,updated_at) VALUES ('{assignment}','{encounter}','{bed}',now(),now(),now());
             INSERT INTO rtt_pathways (id,person_id,target_service,started_at,created_at,updated_at) VALUES ('{pathway}','{person}','Bug8 {tag}',now() - interval '30 days',now(),now());
             INSERT INTO rtt_clock_events (id,pathway_id,kind,event_at,created_at) VALUES ('{}','{pathway}','started',now() - interval '30 days',now());",
            uuid::Uuid::new_v4()
        );
        sqlx::raw_sql(AssertSqlSafe(seed)).execute(&ctx.db).await.expect("seed");

        let ward_page = request.get(&format!("/wards/{ward}")).await;
        let rtt_page = request.get("/rtt").await;

        let cleanup = format!(
            "DELETE FROM rtt_clock_events WHERE pathway_id='{pathway}';
             DELETE FROM rtt_pathways WHERE id='{pathway}';
             DELETE FROM bed_assignments WHERE id='{assignment}';
             DELETE FROM encounters WHERE id='{encounter}';
             DELETE FROM beds WHERE id='{bed}';
             DELETE FROM rooms WHERE id='{room}';
             DELETE FROM wards WHERE id='{ward}';
             DELETE FROM facilities WHERE id='{facility}';"
        );
        sqlx::raw_sql(AssertSqlSafe(cleanup)).execute(&ctx.db).await.expect("cleanup");

        ward_page.assert_status_ok();
        assert!(ward_page.text().contains("Stubfamily"), "ward page: {}", ward_page.text());
        rtt_page.assert_status_ok();
        assert!(rtt_page.text().contains("Stubfamily"), "rtt page: {}", rtt_page.text());
    })
    .await;
}

/// Static files sit inside the middleware, and a missing one is a real `404`.
#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn a_missing_static_file_is_a_404_with_the_security_headers() {
    if database_url().is_none() {
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/static/no-such-file.css").await;
        resp.assert_status_not_found();
        assert_eq!(resp.header("x-content-type-options"), "nosniff");
        assert_eq!(resp.header("x-frame-options"), "DENY");
    })
    .await;
}

/// This service shares the PAS database, so it will not revert or empty it, whoever asks.
#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn the_commands_that_would_destroy_the_shared_database_refuse() {
    if database_url().is_none() {
        return;
    }
    for command in [
        vec!["db", "reset"],
        vec!["db", "down"],
        vec!["db", "truncate"],
    ] {
        let mut cli = Cli::parse_from([&["pas"], command.as_slice()].concat());
        cli.environment = Some("test".to_string());
        let refused = Service::run(cli).await.unwrap_err().to_string();
        assert!(
            refused.contains("will not revert"),
            "{command:?}: {refused}"
        );
    }
    request::<App, _, _>(|request, _ctx| async move {
        let body = request.get("/_health").await.text();
        assert!(body.contains("\"database\":\"ok\""), "{body}");
    })
    .await;
}

/// SQLX-1: every read in `models` runs against rows, and returns what was seeded. sqlx checks
/// SQL and column types at run time, the dashboard swallows read errors, and several reads sit
/// behind an empty guard, so the page tests alone can pass over a broken query.
#[tokio::test]
#[ignore = "requires PostgreSQL (DB_URL); run with `cargo test -- --ignored`"]
#[serial]
async fn every_read_returns_the_rows_seeded_for_it() {
    use patient_administration_system_frontend::models::{
        audit, bed, letter_template, occupancy, outbox, rtt, schedule, ward,
    };
    use sqlx::AssertSqlSafe;

    let url = stub_person_service().await;
    // SAFETY: #[serial] keeps every other test in this binary from running now.
    unsafe { std::env::set_var("PERSON_SERVICE_URL", &url) };
    request::<App, _, _>(|_request, ctx| async move {
        let ids: Vec<uuid::Uuid> = (0..14).map(|_| uuid::Uuid::new_v4()).collect();
        let [
            facility,
            ward_id,
            room,
            occupied,
            free,
            encounter,
            assignment,
            person,
            pathway,
            schedule_id,
            slot,
            template,
            event,
            audit_id,
        ] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13].map(|i| ids[i]);
        let tag = &facility.simple().to_string()[..8];
        let seed = format!(
            "INSERT INTO facilities (id,name,code,address,created_at,updated_at) VALUES ('{facility}','F {tag}','F{tag}','{{}}',now(),now());
             INSERT INTO wards (id,facility_id,name,code,capacity,created_at,updated_at) VALUES ('{ward_id}','{facility}','W {tag}','W{tag}',2,now(),now());
             INSERT INTO rooms (id,ward_id,name,code,created_at,updated_at) VALUES ('{room}','{ward_id}','R {tag}','R{tag}',now(),now());
             INSERT INTO beds (id,room_id,name,code,status,created_at,updated_at) VALUES
               ('{occupied}','{room}','B1 {tag}','B1{tag}','occupied',now(),now()),
               ('{free}','{room}','B2 {tag}','B2{tag}','available',now(),now());
             INSERT INTO encounters (id,person_id,class,status,period_start,created_at,updated_at) VALUES ('{encounter}','{person}','inpatient','in-progress',now(),now(),now());
             INSERT INTO bed_assignments (id,encounter_id,bed_id,assigned_at,created_at,updated_at) VALUES ('{assignment}','{encounter}','{occupied}',now(),now(),now());
             INSERT INTO rtt_pathways (id,person_id,target_service,started_at,created_at,updated_at) VALUES ('{pathway}','{person}','Sqlx {tag}',now() - interval '21 days',now(),now());
             INSERT INTO rtt_clock_events (id,pathway_id,kind,event_at,created_at) VALUES ('{event}','{pathway}','started',now() - interval '21 days',now());
             INSERT INTO schedules (id,owner_kind,owner_id,service_type,created_at,updated_at) VALUES ('{schedule_id}','worker','{person}','Sqlx {tag}',now(),now());
             INSERT INTO slots (id,schedule_id,start_datetime,end_datetime,created_at,updated_at) VALUES ('{slot}','{schedule_id}',now() + interval '1 day',now() + interval '1 day 30 minutes',now(),now());
             INSERT INTO letter_templates (id,name,subject,body_tera,channels,created_at,updated_at) VALUES ('{template}','Sqlx {tag}','Subject','Body','[\"post\",\"email\"]',now(),now());
             INSERT INTO outbox_events (id,event_type,payload) VALUES ('{audit_id}','sqlx.test','{{}}');
             INSERT INTO audit_log (id,entity_type,entity_id,action,at) VALUES ('{audit_id}','sqlx-{tag}','{ward_id}','read',now() + interval '1 hour');"
        );
        sqlx::raw_sql(AssertSqlSafe(seed)).execute(&ctx.db).await.expect("seed");

        let db = &ctx.db;
        let unpublished = outbox::count_unpublished(db).await;
        let recent = audit::list_recent_audit(db, 1).await;
        let wards = ward::list_active_wards(db).await;
        let by_ids = ward::find_wards_by_ids(db, &[ward_id]).await;
        let by_id = ward::find_ward_by_id(db, ward_id).await;
        let missing = ward::find_ward_by_id(db, uuid::Uuid::new_v4()).await;
        let statuses = bed::list_by_ward(db, ward_id).await;
        let available = bed::list_available(db).await;
        let detailed = bed::list_by_ward_detailed(db, ward_id).await;
        let occupants = occupancy::current_occupants_for_beds(db, &[occupied, free]).await;
        let schedules = schedule::list_active_schedules(db).await;
        let slots = schedule::list_free_slots(db, schedule_id, 2).await;
        let templates = letter_template::list_active_templates(db).await;
        let pathways = rtt::list_active_pathways(db).await;

        let cleanup = format!(
            "DELETE FROM audit_log WHERE id='{audit_id}';
             DELETE FROM outbox_events WHERE id='{audit_id}';
             DELETE FROM letter_templates WHERE id='{template}';
             DELETE FROM slots WHERE id='{slot}';
             DELETE FROM schedules WHERE id='{schedule_id}';
             DELETE FROM rtt_clock_events WHERE pathway_id='{pathway}';
             DELETE FROM rtt_pathways WHERE id='{pathway}';
             DELETE FROM bed_assignments WHERE id='{assignment}';
             DELETE FROM encounters WHERE id='{encounter}';
             DELETE FROM beds WHERE room_id='{room}';
             DELETE FROM rooms WHERE id='{room}';
             DELETE FROM wards WHERE id='{ward_id}';
             DELETE FROM facilities WHERE id='{facility}';"
        );
        sqlx::raw_sql(AssertSqlSafe(cleanup)).execute(&ctx.db).await.expect("cleanup");

        assert!(unpublished.expect("outbox") >= 1);
        let recent = recent.expect("audit");
        assert_eq!(recent[0].entity_type, format!("sqlx-{tag}"));
        assert_eq!(recent[0].user_id, None);
        assert!(wards.expect("wards").iter().any(|w| w.id == ward_id));
        assert_eq!(by_ids.expect("wards by id")[0].code, format!("W{tag}"));
        assert_eq!(by_id.expect("ward").expect("the ward").id, ward_id);
        assert!(missing.expect("no ward").is_none());

        let mut statuses: Vec<String> =
            statuses.expect("beds").into_iter().map(|b| b.status).collect();
        statuses.sort();
        assert_eq!(statuses, ["available", "occupied"]);
        let available = available.expect("available beds");
        let picked = available.iter().find(|b| b.id == free).expect("the free bed");
        assert_eq!(picked.room_code, format!("R{tag}"));
        assert_eq!(picked.ward_code, format!("W{tag}"));
        assert!(!available.iter().any(|b| b.id == occupied));
        let detailed = detailed.expect("ward beds");
        assert_eq!(detailed.len(), 2);
        assert_eq!(detailed[0].code, format!("B1{tag}"));

        let occupants = occupants.expect("occupants");
        assert_eq!(occupants.len(), 1);
        assert_eq!(occupants[&occupied].person_id, person);
        assert_eq!(occupants[&occupied].family, "Stubfamily");

        assert!(schedules.expect("schedules").iter().any(|s| s.id == schedule_id));
        let slots = slots.expect("slots");
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].id, slot);
        let templates = templates.expect("templates");
        let template_row = templates.iter().find(|t| t.id == template).expect("the template");
        assert_eq!(template_row.channels, ["post", "email"]);
        let pathways = pathways.expect("pathways");
        let row = pathways.iter().find(|p| p.pathway_id == pathway).expect("the pathway");
        assert_eq!(row.weeks_waiting, 3);
        assert_eq!(row.breach_weeks, 18);
        assert!(!row.is_breaching);
    })
    .await;
}
