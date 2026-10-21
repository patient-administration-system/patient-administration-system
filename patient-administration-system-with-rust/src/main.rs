//! Patient Administration System binary entry point.
//!
//! Loads configuration, initializes observability, connects to the database,
//! builds the application state, and starts the Axum HTTP server.

#![cfg_attr(not(test), forbid(unsafe_code))]
// Production code handles every failure; tests may unwrap (QUAL-12).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
// The entry point stays a short list of steps (QUAL-30).
#![deny(clippy::too_many_lines)]
use std::net::SocketAddr;
use std::sync::Arc;

use patient_administration_system::{
    Error, Result,
    api::ApiDoc,
    api::dashboard,
    api::fhir::fhir_router,
    api::rest::{AppState, RateLimitConfig, RateLimiter, rate_limit_middleware, router},
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
    dotenvy::dotenv().ok();
    let cfg = Config::from_env()?;
    observability::init(&cfg)?;
    tracing::info!(
        "PAS server starting on {}:{}",
        cfg.server_host,
        cfg.server_port
    );

    let db = connect(&cfg.database_url).await?;
    let (person_client, worker_client) = upstream_clients(&cfg)?;
    let publisher = publisher_chain(&cfg, &db, person_client.as_ref(), worker_client.as_ref())?;
    let state = build_state(
        &db,
        publisher.clone(),
        sms_provider(&cfg),
        person_client,
        worker_client,
    );

    // Every timer loop runs under `jobs`: one at a time across replicas, a panic does not end it, and it
    // stops cleanly on SIGTERM (QUAL-2).
    let mut jobs = Jobs::new(db.clone());
    spawn_jobs(&mut jobs, &cfg, &db, &publisher, &state)?;

    let app = build_app(&cfg, state).await?;
    start_mllp(&cfg, &app);
    serve(&cfg, app).await?;
    // Let a job that is mid-run finish (a half-sent reminder step is the worst case) before the process exits.
    jobs.stop(std::time::Duration::from_secs(30)).await;
    Ok(())
}

/// The clients for `person-service` and `worker-service`, each optional.
fn upstream_clients(cfg: &Config) -> Result<(Option<PersonClient>, Option<WorkerClient>)> {
    // Both clients are optional: without a URL the proxy handlers answer 503 and the rest of the PAS runs.
    let upstream_timeout = std::time::Duration::from_secs(cfg.upstream_timeout_secs);
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
    db: &sea_orm::DatabaseConnection,
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
    db: &sea_orm::DatabaseConnection,
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
    db: &sea_orm::DatabaseConnection,
    publisher: &Publisher,
    state: &AppState,
) -> Result<()> {
    spawn_outbox(jobs, cfg, db, publisher);
    spawn_tolerance(jobs, state);
    spawn_noshow(jobs, state);
    spawn_backfill(jobs, state);
    spawn_reminders(jobs, db, state)
}

fn spawn_outbox(
    jobs: &mut Jobs,
    cfg: &Config,
    db: &sea_orm::DatabaseConnection,
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

fn spawn_tolerance(jobs: &mut Jobs, state: &AppState) {
    // Tolerance alerts: every PAS_TOLERANCE_CHECK_MINUTES (default 60; 0 turns it off) check each measure and
    // open an alert for any that is red. It reads only; nothing here acts on a patient.
    let tolerance_minutes: u64 = u64::try_from(patient_administration_system::config::env::int(
        "PAS_TOLERANCE_CHECK_MINUTES",
    ))
    .unwrap_or(60);
    if tolerance_minutes > 0 {
        let tolerance = state.tolerance.clone();
        jobs.spawn(
            "tolerance",
            std::time::Duration::from_secs(tolerance_minutes * 60),
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
}

fn spawn_noshow(jobs: &mut Jobs, state: &AppState) {
    // The no-show probability (CAP-12): an hourly pass that scores booked appointments still to come. A no-op
    // unless PAS_NOSHOW_SCORE=true AND a fairness and a clinical safety review are approved; idempotent within a
    // day. It only adds tasks for people to ring.
    if patient_administration_system::noshow::NoShowService::switched_on() {
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
}

fn spawn_backfill(jobs: &mut Jobs, state: &AppState) {
    // Backfill offers (PILOT-8): PAS_BACKFILL_OFFERS=true offers a released slot to the next patient on
    // standby, and every minute expires offers that ran out and moves on to the next patient.
    if patient_administration_system::backfill::enabled() {
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
}

/// The reminder ladder (CAP-13) or, without it, the single reminder (opt in with PAS_REMINDERS_ENABLED=true,
/// lead time PAS_REMINDER_LEAD_HOURS, default 24; scanned every 5 minutes).
fn spawn_reminders(
    jobs: &mut Jobs,
    db: &sea_orm::DatabaseConnection,
    state: &AppState,
) -> Result<()> {
    // The reminder ladder (CAP-13) replaces the single reminder: PAS_REMINDER_LADDER=true. Steps via
    // PAS_REMINDER_STEPS (default main=7d,second=4d,call=3d,final=24h).
    if patient_administration_system::config::env::flag("PAS_REMINDER_LADDER") {
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
    } else if patient_administration_system::config::env::flag("PAS_REMINDERS_ENABLED") {
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
    );
    match patient_administration_system::api::rest::user_auth::UserAuth::build(auth_cfg).await {
        Ok(Some(auth)) => {
            if auth.is_per_user() {
                tracing::info!(
                    "per-user authentication enabled (PASETO v4.public, verified offline)"
                );
                let every = u64::try_from(patient_administration_system::config::env::int(
                    "PAS_PASETO_KEYS_REFRESH_SECS",
                ))
                .unwrap_or(300);
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

    // Per-IP rate-limit middleware (v0.12). Slotted OUTSIDE bearer auth
    // so brute-force token-guessing is throttled, but INSIDE trace so
    // 429s still get logged. `PAS_RATE_LIMIT_RPM=0` disables.
    if cfg.pas_rate_limit_rpm > 0 {
        tracing::info!(
            "Rate-limit middleware enabled: {} req/min, burst {}",
            cfg.pas_rate_limit_rpm,
            cfg.pas_rate_limit_burst
        );
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_minute: cfg.pas_rate_limit_rpm,
            burst: cfg.pas_rate_limit_burst,
        });
        app = app.layer(axum::middleware::from_fn_with_state(
            limiter,
            rate_limit_middleware,
        ));
    } else {
        tracing::info!("Rate-limit middleware disabled (PAS_RATE_LIMIT_RPM=0)");
    }

    let trace = tower_http::trace::TraceLayer::new_for_http();
    let compress = tower_http::compression::CompressionLayer::new();
    Ok(app.layer(trace).layer(cors_layer(cfg)).layer(compress))
}

/// The optional MLLP TCP listener for HL7 v2 messages; it posts each frame to `app`.
fn start_mllp(cfg: &Config, app: &axum::Router) {
    // Optional MLLP TCP listener for HL7 v2 ADT messages.
    if let Some(bind) = cfg.hl7v2_mllp_bind.clone() {
        tracing::info!("HL7 v2 MLLP listener requested on {bind}");
        let server = patient_administration_system::hl7v2::MllpServer::new(app.clone());
        let bind_for_task = bind.clone();
        tokio::spawn(async move {
            if let Err(e) = server.serve(&bind_for_task).await {
                tracing::error!("MLLP listener exited: {e}");
            }
        });
    }
}

/// Bind and serve until a shutdown signal.
async fn serve(cfg: &Config, app: axum::Router) -> Result<()> {
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
    .with_graceful_shutdown(patient_administration_system::jobs::shutdown_signal())
    .await
    .map_err(|e| Error::internal(format!("serve: {e}")))?;
    Ok(())
}
