//! Patient Administration System binary entry point.
//!
//! Loads configuration, initializes observability, connects to the database,
//! builds the application state, and starts the Axum HTTP server.

#![cfg_attr(not(test), forbid(unsafe_code))]
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
    // Outbox publisher chain (v0.14). Each configured backend (HL7 v2
    // MLLP, webhook URL) becomes a child; CompositePublisher fans the
    // event out to every child. If none is configured, the in-memory
    // publisher takes over so the dispatcher can still mark rows
    // published (it just drops them).
    let mut children: Vec<Arc<dyn patient_administration_system::streaming::EventPublisher>> =
        Vec::new();
    // Hold the MLLP peer config but defer building the publisher until
    // after the upstream clients are constructed below — Phase 3c4
    // hydrates events against `person-service` / `worker-service`.
    let hl7v2_peer = cfg.hl7v2_outbound_peer.clone();
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
    // v0.43.0 SearchEngine bootstrap removed in Phase 2 — Tantivy patient
    // index is gone. Phase 3 may add a search proxy over person-service.

    // SMS provider selection (v0.8). Unknown values fall back to NoopSmsProvider
    // so a typo can't accidentally enable auto-send against a live patient
    // list.
    let sms_provider: Arc<dyn patient_administration_system::communication::SmsProvider> =
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
        };

    // Upstream identity + workforce clients (v0.44.0 Option C, Phase 1).
    // Optional in Phase 1 so existing deployments + tests work unchanged.
    // Phase 5 (final wiring) makes both URLs required at startup with an
    // upstream `/api/health` ping check.
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
            tracing::warn!(
                "PERSON_SERVICE_URL not set; identity proxy handlers will return 503 once Phase 3 lands"
            );
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
            tracing::warn!(
                "WORKER_SERVICE_URL not set; workforce proxy handlers will return 503 once Phase 3 lands"
            );
            None
        }
    };

    // Now that the upstream clients exist, build the MLLP publisher (if
    // configured) with them attached so it can hydrate events against
    // person-service / worker-service.
    if let Some(peer) = hl7v2_peer {
        tracing::info!("HL7 v2 outbound peer configured at {peer}");
        let mut mllp = patient_administration_system::streaming::Hl7v2MllpPublisher::new(
            db.clone(),
            peer,
            "PAS",
            "EMR",
        );
        if let Some(ref c) = person_client {
            mllp = mllp.with_person_client(c.clone());
        }
        if let Some(ref c) = worker_client {
            mllp = mllp.with_worker_client(c.clone());
        }
        children.push(Arc::new(mllp));
    }
    let publisher: Arc<dyn patient_administration_system::streaming::EventPublisher> =
        match children.len() {
            0 => Arc::new(InMemoryEventPublisher::new()),
            1 => children.into_iter().next().unwrap(),
            _ => Arc::new(
                patient_administration_system::streaming::CompositePublisher::new(children),
            ),
        };

    let mut state = AppState::new(db.clone(), publisher.clone()).with_sms_provider(sms_provider);
    // Public address of the patient portal, exposed to message templates as `portal_url`.
    if let Ok(url) = std::env::var("PAS_PORTAL_URL")
        && !url.trim().is_empty()
    {
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
        if std::env::var(var).as_deref() == Ok("log") {
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

    let cors = if cfg.cors_origins.is_empty() {
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
    };
    let trace = tower_http::trace::TraceLayer::new_for_http();
    let compress = tower_http::compression::CompressionLayer::new();

    // Background outbox dispatcher: polls the outbox every 2s.
    let dispatcher_db = db.clone();
    let dispatcher_pub = publisher.clone();
    let dispatcher_max_retries = cfg.pas_outbox_max_retries;
    tokio::spawn(async move {
        patient_administration_system::streaming::dispatcher::run(
            dispatcher_db,
            dispatcher_pub,
            std::time::Duration::from_secs(2),
            dispatcher_max_retries,
        )
        .await;
    });

    // Tolerance alerts: every PAS_TOLERANCE_CHECK_MINUTES (default 60; 0 turns it off) check each measure and
    // open an alert for any that is red. It reads only; nothing here acts on a patient.
    let tolerance_minutes: u64 = std::env::var("PAS_TOLERANCE_CHECK_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    if tolerance_minutes > 0 {
        let tolerance = state.tolerance.clone();
        tokio::spawn(async move {
            let mut tick =
                tokio::time::interval(std::time::Duration::from_secs(tolerance_minutes * 60));
            loop {
                tick.tick().await;
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
        });
    }

    // Appointment reminders: opt in with PAS_REMINDERS_ENABLED=true. Lead
    // time via PAS_REMINDER_LEAD_HOURS (default 24); scanned every 5 minutes.
    // The no-show probability (CAP-12): an hourly pass that scores booked appointments still to come. A no-op
    // unless PAS_NOSHOW_SCORE=true AND a fairness and a clinical safety review are approved; idempotent within a
    // day. It only adds tasks for people to ring.
    if patient_administration_system::noshow::NoShowService::switched_on() {
        let noshow = state.noshow.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                tick.tick().await;
                match noshow.recalculate_all().await {
                    Ok(n) => {
                        tracing::info!(target: "pas::noshow", assessed = n, "no-show pass complete");
                    }
                    Err(e) => tracing::info!(target: "pas::noshow", "no-show pass skipped: {e}"),
                }
            }
        });
    }

    // Backfill offers (PILOT-8): PAS_BACKFILL_OFFERS=true offers a released slot to the next patient on
    // standby, and every minute expires offers that ran out and moves on to the next patient.
    if patient_administration_system::backfill::enabled() {
        let backfill = state.backfill.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                match backfill.tick(chrono::Utc::now()).await {
                    Ok(r) if r.expired > 0 => {
                        tracing::info!(target: "pas::backfill", ?r, "offers moved on");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(target: "pas::backfill", "offer scan failed: {e}"),
                }
            }
        });
    }

    // The reminder ladder (CAP-13) replaces the single reminder: PAS_REMINDER_LADDER=true. Steps via
    // PAS_REMINDER_STEPS (default main=7d,second=4d,call=3d,final=24h).
    if std::env::var("PAS_REMINDER_LADDER").as_deref() == Ok("true") {
        let ladder = patient_administration_system::communication::ladder::LadderService::from_env(
            db.clone(),
            state.communication.clone(),
        )?;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                tick.tick().await;
                match ladder.run_once(chrono::Utc::now()).await {
                    Ok(r) => tracing::info!(target: "pas::ladder", ?r, "ladder scan complete"),
                    Err(e) => tracing::warn!(target: "pas::ladder", "ladder scan failed: {e}"),
                }
            }
        });
    } else if std::env::var("PAS_REMINDERS_ENABLED").as_deref() == Ok("true") {
        let lead = patient_administration_system::communication::reminders::lead_from_hours(
            std::env::var("PAS_REMINDER_LEAD_HOURS").ok().as_deref(),
        )?;
        let reminders =
            patient_administration_system::communication::reminders::ReminderService::new(
                db.clone(),
                state.communication.clone(),
            )
            .with_quiet_hours(
                patient_administration_system::communication::consent::QuietHours::from_env()?,
            );
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                tick.tick().await;
                match reminders.run_once(chrono::Utc::now(), lead).await {
                    Ok(r) => tracing::info!(target: "pas::reminders", ?r, "reminder scan complete"),
                    Err(e) => tracing::warn!(target: "pas::reminders", "reminder scan failed: {e}"),
                }
            }
        });
    }

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
                let every = std::env::var("PAS_PASETO_KEYS_REFRESH_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|s| *s >= 30)
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
    app = app.layer(trace).layer(cors).layer(compress);

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
    .await
    .map_err(|e| Error::internal(format!("serve: {e}")))?;
    Ok(())
}
