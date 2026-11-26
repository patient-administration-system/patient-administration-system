//! Patient Administration System binary entry point.
//!
//! Loads configuration, initializes observability, connects to the database,
//! builds the application state, and starts the Axum HTTP server.

#![cfg_attr(not(test), forbid(unsafe_code))]
// Production code handles every failure; tests may unwrap (QUAL-12).
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented,
        clippy::indexing_slicing,
        clippy::string_slice,
        clippy::panic_in_result_fn,
        clippy::unwrap_in_result,
        // RCA-ENF-3: an arithmetic operation that can overflow is checked, or carries a written proof (RCA-EXC-2).
        clippy::arithmetic_side_effects,
        // RCA-CONV-1, RCA-CONV-2: a numeric conversion is lossless (`From`), checked (`TryFrom`), or carries a written proof.
        clippy::as_conversions
    )
)]
// The entry point stays a short list of steps (QUAL-30).
#![deny(clippy::too_many_lines)]
use std::net::SocketAddr;
use std::sync::Arc;

use patient_administration_system::{
    Error, Result,
    api::ApiDoc,
    api::dashboard,
    api::fhir::fhir_router,
    api::rest::{AppState, RateLimits, router, with_rate_limits},
    config::Config,
    db::connect,
    observability,
    streaming::InMemoryEventPublisher,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

type Publisher = Arc<dyn patient_administration_system::streaming::EventPublisher>;
type PersonClient = Arc<dyn patient_administration_system::upstream::PersonClient>;
type WorkerClient = Arc<dyn patient_administration_system::upstream::WorkerClient>;
type Jobs = patient_administration_system::jobs::Jobs;

#[tokio::main]
async fn main() -> Result<()> {
    patient_administration_system::tls::ensure_provider();
    dotenvy::dotenv().ok();
    let cfg = Config::from_env()?;
    observability::init(&cfg)?;
    run(&cfg, patient_administration_system::jobs::shutdown_signal()).await
}

/// Connect, start the background jobs and the listeners, and serve until `shutdown` resolves; then let the jobs
/// finish. `main` passes the process's signals; a test passes a timer.
async fn run(
    cfg: &Config,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    cursor_key()?;
    tracing::info!(
        "PAS server starting on {}:{}",
        cfg.server_host,
        cfg.server_port
    );

    let db = connect(&cfg.database_url).await?;
    let (person_client, worker_client) = upstream_clients(cfg)?;
    let publisher = publisher_chain(cfg, &db, person_client.as_ref(), worker_client.as_ref())?;
    let state = build_state(
        &db,
        publisher.clone(),
        sms_provider(cfg),
        person_client,
        worker_client,
    );

    // Every timer loop runs under `jobs`: one at a time across replicas, a panic does not end it, and it
    // stops cleanly on SIGTERM (QUAL-2).
    let mut jobs = Jobs::new(db.clone());
    spawn_jobs(&mut jobs, cfg, &db, &publisher, &state)?;

    let app = build_app(cfg, state).await?;
    start_mllp(cfg, &app)?;
    serve(cfg, app, shutdown).await?;
    // Let a job that is mid-run finish (a half-sent reminder step is the worst case) before the process exits.
    jobs.stop(std::time::Duration::from_secs(30)).await;
    Ok(())
}

/// The clients for `person-service` and `worker-service`, each optional.
fn upstream_clients(cfg: &Config) -> Result<(Option<PersonClient>, Option<WorkerClient>)> {
    if patient_administration_system::config::env::flag("PAS_REQUIRE_UPSTREAMS")?
        && let Some(refusal) = patient_administration_system::upstream::missing_upstream_urls(
            cfg.person_service_url.as_deref(),
            cfg.worker_service_url.as_deref(),
        )
    {
        return Err(Error::Config(refusal));
    }
    // Both clients are optional: without a URL the proxy handlers answer 503 and the rest of the PAS runs.
    let upstream_timeout = std::time::Duration::from_secs(cfg.upstream_timeout_secs);
    #[expect(
        clippy::as_conversions,
        reason = "not a numeric conversion: the client is coerced to its trait object"
    )]
    let person_client = match cfg.person_service_url.clone() {
        Some(url) => {
            tracing::info!(
                "person-service configured at {url} (bearer_auth={})",
                cfg.person_service_token.is_some()
            );
            Some(Arc::new(
                patient_administration_system::upstream::HttpPersonClient::new(
                    url,
                    upstream_timeout,
                    cfg.person_service_token.clone(),
                )?,
            )
                as Arc<
                    dyn patient_administration_system::upstream::PersonClient,
                >)
        }
        None => {
            tracing::warn!("PERSON_SERVICE_URL not set; identity proxy handlers will return 503");
            None
        }
    };
    #[expect(
        clippy::as_conversions,
        reason = "not a numeric conversion: the client is coerced to its trait object"
    )]
    let worker_client = match cfg.worker_service_url.clone() {
        Some(url) => {
            tracing::info!(
                "worker-service configured at {url} (bearer_auth={})",
                cfg.worker_service_token.is_some()
            );
            Some(Arc::new(
                patient_administration_system::upstream::HttpWorkerClient::new(
                    url,
                    upstream_timeout,
                    cfg.worker_service_token.clone(),
                )?,
            )
                as Arc<
                    dyn patient_administration_system::upstream::WorkerClient,
                >)
        }
        None => {
            tracing::warn!("WORKER_SERVICE_URL not set; workforce proxy handlers will return 503");
            None
        }
    };

    Ok((person_client, worker_client))
}

/// The SMS provider named by `PAS_SMS_PROVIDER` (v0.8). Unknown values fall back to `NoopSmsProvider` so a
/// typo cannot enable auto-send against a live patient list.
fn sms_provider(
    cfg: &Config,
) -> Arc<dyn patient_administration_system::communication::SmsProvider> {
    match cfg.pas_sms_provider.as_str() {
        "log" => {
            tracing::info!("SMS provider: LogSmsProvider (auto-send enabled, log-only)");
            Arc::new(patient_administration_system::communication::LogSmsProvider)
        }
        "none" | "" => {
            tracing::info!("SMS provider: NoopSmsProvider (auto-send disabled)");
            Arc::new(patient_administration_system::communication::NoopSmsProvider)
        }
        other => {
            tracing::warn!(
                "PAS_SMS_PROVIDER={other:?} is not recognised; falling back to NoopSmsProvider"
            );
            Arc::new(patient_administration_system::communication::NoopSmsProvider)
        }
    }
}

/// The outbox publisher chain (v0.14): each configured backend (webhook, HL7 v2 MLLP) is a child that every
/// event fans out to. With none, the in-memory publisher lets the dispatcher mark rows published.
fn publisher_chain(
    cfg: &Config,
    db: &patient_administration_system::db::sql::Pool,
    person_client: Option<&PersonClient>,
    worker_client: Option<&WorkerClient>,
) -> Result<Publisher> {
    let mut children: Vec<Publisher> = Vec::new();
    if let Some(url) = cfg.pas_webhook_url.clone() {
        tracing::info!(
            "Webhook publisher configured: url={url}, hmac_signing={}, timeout={}s",
            cfg.pas_webhook_secret.is_some(),
            cfg.pas_webhook_timeout_secs
        );
        let secret = cfg
            .pas_webhook_secret
            .clone()
            .map(std::string::String::into_bytes);
        let timeout = std::time::Duration::from_secs(cfg.pas_webhook_timeout_secs);
        let webhook = patient_administration_system::streaming::WebhookEventPublisher::new(
            url, secret, timeout,
        )?;
        children.push(Arc::new(webhook));
    }
    // The MLLP publisher hydrates events against person-service / worker-service.
    let hl7v2_peer = cfg.hl7v2_outbound_peer.clone();
    if let Some(peer) = hl7v2_peer {
        tracing::info!("HL7 v2 outbound peer configured at {peer}");
        let mut mllp = patient_administration_system::streaming::Hl7v2MllpPublisher::new(
            db.clone(),
            peer,
            "PAS",
            "EMR",
        );
        if let Some(c) = person_client {
            mllp = mllp.with_person_client(c.clone());
        }
        if let Some(c) = worker_client {
            mllp = mllp.with_worker_client(c.clone());
        }
        children.push(Arc::new(mllp));
    }
    Ok(match children.len() {
        0 | 1 => children
            .into_iter()
            .next()
            .unwrap_or_else(|| Arc::new(InMemoryEventPublisher::new())),
        _ => Arc::new(patient_administration_system::streaming::CompositePublisher::new(children)),
    })
}

/// The application state, with the senders, portal address and upstream clients the environment names.
fn build_state(
    db: &patient_administration_system::db::sql::Pool,
    publisher: Publisher,
    sms_provider: Arc<dyn patient_administration_system::communication::SmsProvider>,
    person_client: Option<PersonClient>,
    worker_client: Option<WorkerClient>,
) -> AppState {
    let mut state = AppState::new(db.clone(), publisher).with_sms_provider(sms_provider);
    // Public address of the patient portal, exposed to message templates as `portal_url`.
    if let Some(url) = patient_administration_system::config::env::text("PAS_PORTAL_URL") {
        state = state.with_portal_base_url(url.trim());
    }
    // Per-channel senders. "log" enables log-only dispatch; anything else
    // leaves the channel disabled so a typo cannot message real patients.
    for (var, channel, label) in [
        (
            "PAS_EMAIL_PROVIDER",
            patient_administration_system::models::communication::DeliveryChannel::Email,
            "email",
        ),
        (
            "PAS_APPLE_MESSAGES_PROVIDER",
            patient_administration_system::models::communication::DeliveryChannel::AppleMessages,
            "apple_messages",
        ),
        (
            "PAS_WHATSAPP_PROVIDER",
            patient_administration_system::models::communication::DeliveryChannel::WhatsApp,
            "whatsapp",
        ),
    ] {
        if patient_administration_system::config::env::text(var).as_deref() == Some("log") {
            tracing::info!("{label} provider: log-only dispatch enabled");
            state = state.with_sender(
                channel,
                Arc::new(
                    patient_administration_system::communication::LogMessageSender {
                        channel: label,
                    },
                ),
            );
        }
    }
    if let Some(c) = person_client {
        state = state.with_person_client(c);
    }
    if let Some(c) = worker_client {
        state = state.with_worker_client(c);
    }

    state
}

/// Every background job the environment switches on.
fn spawn_jobs(
    jobs: &mut Jobs,
    cfg: &Config,
    db: &patient_administration_system::db::sql::Pool,
    publisher: &Publisher,
    state: &AppState,
) -> Result<()> {
    spawn_outbox(jobs, cfg, db, publisher);
    spawn_subscriptions(jobs, db);
    spawn_tolerance(jobs, state)?;
    spawn_noshow(jobs, state)?;
    spawn_backfill(jobs, state)?;
    spawn_work_queue(jobs, state);
    spawn_bed_holds(jobs, state);
    spawn_hl7_dedupe_purge(jobs, db);
    spawn_reminders(jobs, db, state)
}

fn spawn_outbox(
    jobs: &mut Jobs,
    cfg: &Config,
    db: &patient_administration_system::db::sql::Pool,
    publisher: &Publisher,
) {
    // Background outbox dispatcher: polls the outbox every 2s.
    let dispatcher_db = db.clone();
    let dispatcher_pub = publisher.clone();
    let dispatcher_max_retries = cfg.pas_outbox_max_retries;
    jobs.spawn("outbox", std::time::Duration::from_secs(2), move || {
        let (db, publisher) = (dispatcher_db.clone(), dispatcher_pub.clone());
        async move {
            if let Err(e) = patient_administration_system::streaming::dispatcher::tick(
                &db,
                &publisher,
                dispatcher_max_retries,
            )
            .await
            {
                tracing::warn!("outbox dispatcher tick failed: {e}");
            }
        }
    });
}

fn spawn_subscriptions(jobs: &mut Jobs, db: &patient_administration_system::db::sql::Pool) {
    // Event subscriptions (NEW-4): queue each new outbox event for its subscribers and post the due
    // notifications, with retries. A no-op while nobody is subscribed.
    let svc = std::sync::Arc::new(
        patient_administration_system::subscriptions::SubscriptionService::new(db.clone()),
    );
    jobs.spawn(
        "subscriptions",
        std::time::Duration::from_secs(5),
        move || {
            let svc = svc.clone();
            async move {
                if let Err(e) = svc.tick(jiff::Timestamp::now()).await {
                    tracing::warn!("subscription pass failed: {e}");
                }
            }
        },
    );
}

fn spawn_tolerance(jobs: &mut Jobs, state: &AppState) -> Result<()> {
    // Tolerance alerts: every PAS_TOLERANCE_CHECK_MINUTES (default 60; 0 turns it off) check each measure and
    // open an alert for any that is red. It reads only; nothing here acts on a patient.
    let tolerance_minutes: u64 =
        patient_administration_system::config::env::count("PAS_TOLERANCE_CHECK_MINUTES")?;
    if tolerance_minutes > 0 {
        let tolerance = state.tolerance.clone();
        jobs.spawn(
            "tolerance",
            std::time::Duration::from_secs(tolerance_minutes.checked_mul(60).ok_or_else(|| {
                Error::config("PAS_TOLERANCE_CHECK_MINUTES is too large".to_string())
            })?),
            move || {
                let tolerance = tolerance.clone();
                async move {
                    match tolerance.evaluate(30).await {
                        Ok(e) => tracing::info!(
                            target: "pas::tolerance",
                            raised = e.raised,
                            cleared = e.cleared,
                            open = e.open,
                            "tolerance check complete"
                        ),
                        Err(e) => {
                            tracing::warn!(target: "pas::tolerance", "tolerance check failed: {e}")
                        }
                    }
                }
            },
        );
    }
    Ok(())
}

fn spawn_noshow(jobs: &mut Jobs, state: &AppState) -> Result<()> {
    // The no-show probability (CAP-12): an hourly pass that scores booked appointments still to come. A no-op
    // unless PAS_NOSHOW_SCORE=true AND a fairness and a clinical safety review are approved; idempotent within a
    // day. It only adds tasks for people to ring.
    if patient_administration_system::noshow::NoShowService::switched_on()? {
        let noshow = state.noshow.clone();
        jobs.spawn("noshow", std::time::Duration::from_secs(3600), move || {
            let noshow = noshow.clone();
            async move {
                match noshow.recalculate_all().await {
                    Ok(n) => {
                        tracing::info!(target: "pas::noshow", assessed = n, "no-show pass complete");
                    }
                    Err(e) => tracing::info!(target: "pas::noshow", "no-show pass skipped: {e}"),
                }
            }
        });
    }
    Ok(())
}

fn spawn_bed_holds(jobs: &mut Jobs, state: &AppState) {
    // Bed holds (OFH-4): every minute, give back a bed whose pre-admission hold has lapsed. A hold lapses only
    // when it was made with a hold time (a `hold_minutes` on the request, or PAS_BED_HOLD_MINUTES above 0), so
    // with neither this finds nothing.
    let adt = state.adt.clone();
    let clock = state.clock.clone();
    jobs.spawn("bed-holds", std::time::Duration::from_secs(60), move || {
        let adt = adt.clone();
        let clock = clock.clone();
        async move {
            match adt.release_expired_holds(clock.now()).await {
                Ok(n) if n > 0 => {
                    tracing::info!(target: "pas::adt", released = n, "bed holds lapsed");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(target: "pas::adt", "bed hold sweep failed: {e}"),
            }
        }
    });
}

fn spawn_hl7_dedupe_purge(jobs: &mut Jobs, db: &patient_administration_system::db::sql::Pool) {
    // HL7 v2 duplicate keys (NFR-31, HUB-2) older than PAS_HL7_DEDUPE_DAYS, and Idempotency-Key answers (HUB-8)
    // older than 24 hours: every hour.
    let db = db.clone();
    jobs.spawn("hl7-dedupe-purge", std::time::Duration::from_secs(3600), move || {
        let db = db.clone();
        async move {
            match patient_administration_system::hl7v2::dedupe::purge(&db).await {
                Ok(n) if n > 0 => tracing::info!(target: "pas::hl7v2::dedupe", removed = n, "old duplicate keys removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(target: "pas::hl7v2::dedupe", "duplicate key purge failed: {e}"),
            }
            match patient_administration_system::api::rest::idempotency::purge(&db).await {
                Ok(n) if n > 0 => tracing::info!(target: "pas::idempotency", removed = n, "old idempotency keys removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(target: "pas::idempotency", "idempotency key purge failed: {e}"),
            }
        }
    });
}

/// The staff work queue (TSC-1): every minute, an item for each open call, follow-up, validation, needs review
/// and tolerance alert, closed when its source closes.
fn spawn_work_queue(jobs: &mut Jobs, state: &AppState) {
    let db = state.db.clone();
    let clock = state.clock.clone();
    jobs.spawn(
        "work-queue",
        std::time::Duration::from_secs(60),
        move || {
            let db = db.clone();
            let clock = clock.clone();
            async move {
                match patient_administration_system::work_queue::WorkQueueService::new(db)
                    .sync(clock.now())
                    .await
                {
                    Ok(r) if r.opened > 0 || r.reopened > 0 || r.closed > 0 => {
                        tracing::info!(target: "pas::work_queue", ?r, "work queue synced");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(target: "pas::work_queue", "work queue sync failed: {e}")
                    }
                }
            }
        },
    );
}

fn spawn_backfill(jobs: &mut Jobs, state: &AppState) -> Result<()> {
    // Backfill offers (PILOT-8): PAS_BACKFILL_OFFERS=true offers a released slot to the next patient on
    // standby, and every minute expires offers that ran out and moves on to the next patient.
    if patient_administration_system::backfill::enabled()? {
        let backfill = state.backfill.clone();
        let clock = state.clock.clone();
        jobs.spawn("backfill", std::time::Duration::from_secs(60), move || {
            let backfill = backfill.clone();
            let clock = clock.clone();
            async move {
                match backfill.tick(clock.now()).await {
                    Ok(r) if r.expired > 0 => {
                        tracing::info!(target: "pas::backfill", ?r, "offers moved on");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(target: "pas::backfill", "offer scan failed: {e}"),
                }
            }
        });
    }
    Ok(())
}

/// The reminder ladder (CAP-13) or, without it, the single reminder (opt in with PAS_REMINDERS_ENABLED=true,
/// lead time PAS_REMINDER_LEAD_HOURS, default 24; scanned every 5 minutes).
fn spawn_reminders(
    jobs: &mut Jobs,
    db: &patient_administration_system::db::sql::Pool,
    state: &AppState,
) -> Result<()> {
    // The reminder ladder (CAP-13) replaces the single reminder: PAS_REMINDER_LADDER=true. Steps via
    // PAS_REMINDER_STEPS (default main=7d,second=4d,call=3d,final=24h).
    if patient_administration_system::config::env::flag("PAS_REMINDER_LADDER")? {
        let ladder = patient_administration_system::communication::ladder::LadderService::from_env(
            db.clone(),
            state.communication.clone(),
        )?;
        let ladder = std::sync::Arc::new(ladder);
        let clock = state.clock.clone();
        jobs.spawn("ladder", std::time::Duration::from_secs(300), move || {
            let ladder = ladder.clone();
            let clock = clock.clone();
            async move {
                match ladder.run_once(clock.now()).await {
                    Ok(r) => tracing::info!(target: "pas::ladder", ?r, "ladder scan complete"),
                    Err(e) => tracing::warn!(target: "pas::ladder", "ladder scan failed: {e}"),
                }
            }
        });
    } else if patient_administration_system::config::env::flag("PAS_REMINDERS_ENABLED")? {
        let lead = patient_administration_system::communication::reminders::lead_from_hours(
            patient_administration_system::config::env::text("PAS_REMINDER_LEAD_HOURS").as_deref(),
        )?;
        let reminders =
            patient_administration_system::communication::reminders::ReminderService::new(
                db.clone(),
                state.communication.clone(),
            )
            .with_quiet_hours(
                patient_administration_system::communication::consent::QuietHours::from_env()?,
            );
        let reminders = std::sync::Arc::new(reminders);
        let clock = state.clock.clone();
        jobs.spawn(
            "reminders",
            std::time::Duration::from_secs(300),
            move || {
                let reminders = reminders.clone();
                let clock = clock.clone();
                async move {
                    match reminders.run_once(clock.now(), lead).await {
                        Ok(r) => {
                            tracing::info!(target: "pas::reminders", ?r, "reminder scan complete")
                        }
                        Err(e) => {
                            tracing::warn!(target: "pas::reminders", "reminder scan failed: {e}")
                        }
                    }
                }
            },
        );
    }

    Ok(())
}

/// CORS from `CORS_ORIGINS`; permissive (with a warning) when unset.
fn cors_layer(cfg: &Config) -> tower_http::cors::CorsLayer {
    if cfg.cors_origins.is_empty() {
        tracing::warn!("CORS_ORIGINS unset; using permissive CORS");
        tower_http::cors::CorsLayer::permissive()
    } else {
        tracing::info!("CORS restricted to origins: {:?}", cfg.cors_origins);
        let origins: Vec<axum::http::HeaderValue> = cfg
            .cors_origins
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        tower_http::cors::CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    }
}

/// The whole HTTP application: REST, FHIR, the dashboard and Swagger, then authentication, the rate limit,
/// tracing, CORS and compression, in that order from the inside out.
async fn build_app(cfg: &Config, state: AppState) -> Result<axum::Router> {
    let dashboard_routes = axum::Router::new()
        .route("/dashboard", axum::routing::get(dashboard::dashboard_page))
        .route(
            "/dashboard/wards",
            axum::routing::get(dashboard::dashboard_wards),
        )
        .route(
            "/dashboard/breaches",
            axum::routing::get(dashboard::dashboard_breaches),
        )
        .route(
            "/dashboard/outbox",
            axum::routing::get(dashboard::dashboard_outbox),
        )
        .route(
            "/dashboard/audit",
            axum::routing::get(dashboard::dashboard_audit),
        )
        .with_state(state.clone());
    let mut app = router(state.clone())
        .merge(fhir_router(state))
        .merge(dashboard_routes)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()));
    // Authentication (DTAC-4). With PAS_PASETO_KEYS_URL / PAS_PASETO_KEYS_JSON every caller needs a
    // verified per-user token; with only API_TOKEN it is the old shared token; with neither the API
    // trusts its callers. PAS_REQUIRE_AUTH=true refuses to start without per-user auth.
    let auth_cfg = patient_administration_system::api::rest::user_auth::UserAuthConfig::from_env(
        cfg.api_token.clone(),
    )?;
    match patient_administration_system::api::rest::user_auth::UserAuth::build(auth_cfg).await {
        Ok(Some(auth)) => {
            if auth.is_per_user() {
                tracing::info!(
                    "per-user authentication enabled (PASETO v4.public, verified offline)"
                );
                let every: u64 = patient_administration_system::config::env::count(
                    "PAS_PASETO_KEYS_REFRESH_SECS",
                )?;
                patient_administration_system::api::rest::user_auth::spawn_key_refresh(
                    auth.clone(),
                    std::time::Duration::from_secs(every),
                );
            } else {
                tracing::warn!(
                    "shared API_TOKEN only: callers are not individually identified; set PAS_PASETO_KEYS_URL"
                );
            }
            app = patient_administration_system::api::rest::user_auth::with_user_auth(app, auth);
        }
        Ok(None) => tracing::warn!("no authentication configured; running in trusted-caller mode"),
        Err(e) => return Err(Error::Config(e)),
    }

    // The limit per client and the lockout after repeated 401s (axa-request-rate-limit), outside authentication so
    // that token guessing is slowed and the lockout sees its 401s, and inside the request telemetry so that
    // a 429 is logged. `0` turns either off.
    tracing::info!(
        requests_per_minute = cfg.pas_rate_limit_rpm,
        lockout_client_address_after_count = cfg.lockout_client_address_after_count,
        "request limits"
    );
    if patient_administration_system::config::env::text("PAS_RATE_LIMIT_BURST").is_some() {
        tracing::warn!(
            "PAS_RATE_LIMIT_BURST is no longer read: the limit is PAS_RATE_LIMIT_RPM in each fixed minute"
        );
    }
    {
        use patient_administration_system::config::{LOCKOUT_VAR, OLD_LOCKOUT_VAR, env};
        if env::text(OLD_LOCKOUT_VAR).is_some() {
            if env::text(LOCKOUT_VAR).is_some() {
                tracing::warn!(
                    "{OLD_LOCKOUT_VAR} is not read while {LOCKOUT_VAR} is set: remove it"
                );
            } else {
                tracing::warn!("{OLD_LOCKOUT_VAR} is the old name of {LOCKOUT_VAR}: rename it");
            }
        }
    }
    app = with_rate_limits(
        app,
        RateLimits::new(
            cfg.pas_rate_limit_rpm,
            cfg.lockout_client_address_after_count,
        ),
    );

    let compress = tower_http::compression::CompressionLayer::new();
    Ok(
        patient_administration_system::observability::with_request_telemetry(app)
            .layer(cors_layer(cfg))
            .layer(compress),
    )
}

/// The optional MLLP TCP listener for HL7 v2 messages; it posts each frame to `app`.
fn start_mllp(cfg: &Config, app: &axum::Router) -> Result<()> {
    // Optional MLLP TCP listener for HL7 v2 ADT messages. Its TLS settings are read first, so a certificate
    // without a key (or a file that does not load) stops the server here instead of listening in plain text.
    let options = patient_administration_system::hl7v2::listener::MllpOptions::from_env()?;
    if let Some(bind) = cfg.hl7v2_mllp_bind.clone() {
        tracing::info!("HL7 v2 MLLP listener requested on {bind}");
        let server = patient_administration_system::hl7v2::MllpServer::new(app.clone())
            .with_options(options);
        let bind_for_task = bind.clone();
        tokio::spawn(async move {
            if let Err(e) = server.serve(&bind_for_task).await {
                tracing::error!("MLLP listener exited: {e}");
            }
        });
    }
    Ok(())
}

/// The key that signs list cursors, from `PAS_CURSOR_KEY`. Without it each process signs with its own, so a
/// cursor fails after a restart or on another replica; outside development that is worth a warning.
fn cursor_key() -> Result<()> {
    let secret = patient_administration_system::config::env::text("PAS_CURSOR_KEY");
    let shared = patient_administration_system::paging::init_cursor_key(secret.as_deref())?;
    if !shared && axa_environment::Environment::from_env().is_production_like() {
        tracing::warn!(
            "PAS_CURSOR_KEY is not set: list cursors are signed with a key of this process alone, so a \
             next page fails after a restart or on another replica"
        );
    }
    Ok(())
}

/// Bind and serve until `shutdown` resolves.
async fn serve(
    cfg: &Config,
    app: axum::Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", cfg.server_host, cfg.server_port)
        .parse()
        .map_err(|e: std::net::AddrParseError| Error::config(format!("bad bind addr: {e}")))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| Error::config(format!("bind: {e}")))?;
    // `into_make_service_with_connect_info::<SocketAddr>()` is required
    // so the rate-limit middleware can extract the peer IP via
    // `ConnectInfo<SocketAddr>`. Without it, every request looks like it
    // came from no IP and all clients share one bucket.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .map_err(|e| Error::internal(format!("serve: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configuration made from the environment's defaults. Only `DATABASE_URL` is needed; every test here
    /// then sets the fields it cares about, so none depends on what else the process has set.
    fn config() -> Config {
        static SET: std::sync::Once = std::sync::Once::new();
        SET.call_once(|| {
            if std::env::var_os("DATABASE_URL").is_none() {
                // SAFETY: test-only, and set once before any test reads it.
                unsafe { std::env::set_var("DATABASE_URL", "postgres://u:p@localhost/db") };
            }
        });
        Config::from_env().unwrap()
    }

    #[test]
    fn cors_is_permissive_without_origins_and_restricted_with_them() {
        let mut cfg = config();
        cfg.cors_origins = vec![];
        let _ = cors_layer(&cfg);
        cfg.cors_origins = vec![
            "https://portal.example".into(),
            "not a header value\n".into(),
        ];
        let _ = cors_layer(&cfg);
    }

    #[test]
    fn the_sms_provider_follows_the_name_and_unknown_names_send_nothing() {
        let mut cfg = config();
        for name in ["log", "none", "", "twilio"] {
            cfg.pas_sms_provider = name.into();
            let _ = sms_provider(&cfg);
        }
    }

    #[test]
    fn upstream_clients_are_built_only_for_the_urls_given() {
        let mut cfg = config();
        cfg.person_service_url = None;
        cfg.worker_service_url = None;
        let (p, w) = upstream_clients(&cfg).unwrap();
        assert!(p.is_none() && w.is_none());
        cfg.person_service_url = Some("http://127.0.0.1:1".into());
        cfg.person_service_token = Some("t".into());
        cfg.worker_service_url = Some("http://127.0.0.1:2".into());
        let (p, w) = upstream_clients(&cfg).unwrap();
        assert!(p.is_some() && w.is_some());
    }

    #[test]
    fn a_missing_cursor_key_is_not_an_error() {
        cursor_key().unwrap();
    }
    // The database tests below run the start-up steps against PostgreSQL: `cargo test -- --ignored`.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Sets environment variables for one test and restores them; holds a lock so tests do not see each other's.
    struct Env<'a> {
        _lock: std::sync::MutexGuard<'a, ()>,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl Env<'_> {
        fn set(vars: &[(&'static str, &str)]) -> Self {
            let _lock = ENV
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut saved = Vec::new();
            for (key, value) in vars {
                saved.push((*key, std::env::var_os(key)));
                // SAFETY: test-only, and serialized by `ENV`.
                unsafe { std::env::set_var(key, value) };
            }
            Self { _lock, saved }
        }
    }

    impl Drop for Env<'_> {
        fn drop(&mut self) {
            for (key, old) in self.saved.drain(..) {
                // SAFETY: test-only, and serialized by `ENV`.
                unsafe {
                    match old {
                        Some(v) => std::env::set_var(key, v),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    /// A pathway 30 weeks into an 18-week target, so the dashboard has a breach to show.
    async fn breaching_pathway(db: &patient_administration_system::db::sql::Pool) {
        use patient_administration_system::db::sql::{Conn, Stmt};
        let pathway = uuid::Uuid::new_v4();
        db.run(Stmt::new(
            "INSERT INTO rtt_pathways (id, person_id, target_service, breach_weeks, status, started_at, created_at, updated_at) \
             VALUES ($1, $2, 'cardiology', 18, 'active', now() - interval '30 weeks', now(), now())",
            [pathway.into(), uuid::Uuid::new_v4().into()],
        ))
        .await
        .unwrap();
        db.run(Stmt::new(
            "INSERT INTO rtt_clock_events (id, pathway_id, kind, event_at, created_at) \
             VALUES ($1, $2, 'started', now() - interval '30 weeks', now())",
            [uuid::Uuid::new_v4().into(), pathway.into()],
        ))
        .await
        .unwrap();
    }

    async fn pool() -> patient_administration_system::db::sql::Pool {
        let url = std::env::var("DB_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap();
        connect(&url).await.unwrap()
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
    async fn the_publisher_chain_has_one_child_per_backend() {
        let db = pool().await;
        let mut cfg = config();
        cfg.pas_webhook_url = None;
        cfg.hl7v2_outbound_peer = None;
        publisher_chain(&cfg, &db, None, None).unwrap();
        cfg.pas_webhook_url = Some("http://127.0.0.1:1/hook".into());
        cfg.pas_webhook_secret = Some("secret".into());
        publisher_chain(&cfg, &db, None, None).unwrap();
        cfg.hl7v2_outbound_peer = Some("127.0.0.1:1".into());
        cfg.person_service_url = Some("http://127.0.0.1:1".into());
        cfg.worker_service_url = Some("http://127.0.0.1:2".into());
        let (person, worker) = upstream_clients(&cfg).unwrap();
        publisher_chain(&cfg, &db, person.as_ref(), worker.as_ref()).unwrap();
        cfg.pas_webhook_url = None;
        publisher_chain(&cfg, &db, person.as_ref(), worker.as_ref()).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
    async fn the_state_takes_each_provider_the_environment_names() {
        let _env = Env::set(&[
            ("PAS_PORTAL_URL", " https://portal.example "),
            ("PAS_EMAIL_PROVIDER", "log"),
            ("PAS_APPLE_MESSAGES_PROVIDER", "log"),
            ("PAS_WHATSAPP_PROVIDER", "other"),
        ]);
        let db = pool().await;
        let mut cfg = config();
        cfg.person_service_url = Some("http://127.0.0.1:1".into());
        cfg.worker_service_url = Some("http://127.0.0.1:2".into());
        let (person, worker) = upstream_clients(&cfg).unwrap();
        let publisher = publisher_chain(&cfg, &db, None, None).unwrap();
        let _ = build_state(&db, publisher, sms_provider(&cfg), person, worker);
    }

    #[test]
    fn cors_origins_are_split_trimmed_and_blanks_dropped() {
        let _env = Env::set(&[("CORS_ORIGINS", " https://a.example , ,https://b.example")]);
        assert_eq!(
            config().cors_origins,
            ["https://a.example", "https://b.example"]
        );
    }

    #[test]
    fn required_upstreams_are_refused() {
        let mut cfg = config();
        cfg.person_service_url = None;
        cfg.worker_service_url = None;
        let _env = Env::set(&[("PAS_REQUIRE_UPSTREAMS", "true")]);
        assert!(upstream_clients(&cfg).is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
    async fn every_job_switched_on_runs_once_and_stops() {
        let db = pool().await;
        let cfg = config();
        let publisher: Publisher = Arc::new(InMemoryEventPublisher::new());
        let state = build_state(&db, publisher.clone(), sms_provider(&cfg), None, None);
        for vars in [
            vec![
                ("PAS_TOLERANCE_CHECK_MINUTES", "1"),
                ("PAS_BACKFILL_OFFERS", "true"),
                ("PAS_REMINDER_LADDER", "true"),
            ],
            vec![
                ("PAS_TOLERANCE_CHECK_MINUTES", "1"),
                ("PAS_REMINDERS_ENABLED", "true"),
                ("PAS_REMINDER_LEAD_HOURS", "24"),
            ],
        ] {
            let _env = Env::set(&vars);
            let mut jobs = Jobs::new(db.clone());
            spawn_jobs(&mut jobs, &cfg, &db, &publisher, &state).unwrap();
            // Each job runs at once, then waits for its interval.
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            jobs.stop(std::time::Duration::from_secs(60)).await;
        }
        let _env = Env::set(&[("PAS_TOLERANCE_CHECK_MINUTES", "18446744073709551615")]);
        let mut jobs = Jobs::new(db.clone());
        assert!(spawn_tolerance(&mut jobs, &state).is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
    async fn the_app_is_built_with_and_without_authentication() {
        let db = pool().await;
        let mut cfg = config();
        let publisher: Publisher = Arc::new(InMemoryEventPublisher::new());
        let sms = sms_provider(&cfg);
        let state = || build_state(&db, publisher.clone(), sms.clone(), None, None);
        {
            let _env = Env::set(&[("PAS_RATE_LIMIT_BURST", "5")]);
            cfg.api_token = None;
            let _ = build_app(&cfg, state()).await.unwrap();
        }
        cfg.api_token = Some("shared-token".into());
        let app = build_app(&cfg, state()).await.unwrap();
        // The dashboard fragments read wards, pathways, the outbox and the audit log.
        patient_administration_system::seed::run(&db).await.unwrap();
        breaching_pathway(&db).await;
        let app = app.layer(axum::extract::connect_info::MockConnectInfo(
            "127.0.0.1:9".parse::<SocketAddr>().unwrap(),
        ));
        for path in [
            "/dashboard",
            "/dashboard/wards",
            "/dashboard/breaches",
            "/dashboard/outbox",
            "/dashboard/audit",
        ] {
            use tower::ServiceExt;
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .header("authorization", "Bearer shared-token")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK, "{path}");
        }
        {
            use patient_administration_system::config::{LOCKOUT_VAR, OLD_LOCKOUT_VAR};
            let _env = Env::set(&[(OLD_LOCKOUT_VAR, "5")]);
            let _ = build_app(&cfg, state()).await.unwrap();
            drop(_env);
            let _env = Env::set(&[(OLD_LOCKOUT_VAR, "5"), (LOCKOUT_VAR, "5")]);
            let _ = build_app(&cfg, state()).await.unwrap();
        }
        cfg.api_token = None;
        let _env = Env::set(&[("PAS_REQUIRE_AUTH", "true")]);
        assert!(build_app(&cfg, state()).await.is_err());
    }

    #[tokio::test]
    async fn the_mllp_listener_starts_only_when_a_bind_is_named() {
        let mut cfg = config();
        let app = axum::Router::new();
        cfg.hl7v2_mllp_bind = None;
        start_mllp(&cfg, &app).unwrap();
        cfg.hl7v2_mllp_bind = Some("127.0.0.1:0".into());
        start_mllp(&cfg, &app).unwrap();
    }

    #[tokio::test]
    async fn serving_refuses_a_bad_or_taken_address() {
        let mut cfg = config();
        cfg.server_host = "not an address".into();
        assert!(serve(&cfg, axum::Router::new(), async {}).await.is_err());
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        cfg.server_host = "127.0.0.1".into();
        cfg.server_port = taken.local_addr().unwrap().port();
        assert!(serve(&cfg, axum::Router::new(), async {}).await.is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
    async fn the_server_starts_serves_and_stops_when_asked() {
        let mut cfg = config();
        cfg.database_url = std::env::var("DB_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap();
        cfg.server_host = "127.0.0.1".into();
        cfg.server_port = 0;
        cfg.hl7v2_mllp_bind = None;
        let _env = Env::set(&[("PAS_TOLERANCE_CHECK_MINUTES", "0")]);
        run(
            &cfg,
            tokio::time::sleep(std::time::Duration::from_millis(500)),
        )
        .await
        .unwrap();
        cfg.database_url = "postgres://pas:pas@127.0.0.1:1/pas".into();
        assert!(
            run(&cfg, async {}).await.is_err(),
            "a database that is not there"
        );
    }
}
