//! The application: its configuration, its routes and its command line, as plain functions.
//!
//! This service shares the PAS database and never owns it, so it does not migrate on boot, and
//! `db reset`, `db down` and `db truncate` refuse. Migrations are applied by `pas-migrate`.

use axa::{
    AppContext, Environment, Error, Result,
    cli::{self, Cli, Command, DbCommand},
    config::{Config, Defaults, MB},
    middleware,
    routes::AppRoutes,
    server,
};
use axum::{Extension, Router, middleware::from_fn};
use tower_http::services::ServeDir;

use crate::{controllers, views::Views};

/// Served at `/static`. Files may be absent: a service with no static files still boots.
const STATIC_FOLDER: &str = "assets/static";

/// What the pages load: the layout's `<style>` block and `style` attributes, HTMX from unpkg (pinned
/// by its integrity hash), and HTMX's panel refreshes. axa's default policy suits a JSON API and
/// would block all of these. `HTTP_CSP` replaces it.
const PAGE_CSP: &str = "default-src 'none'; script-src https://unpkg.com; style-src 'unsafe-inline'; \
                        connect-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; \
                        frame-ancestors 'none'";

pub struct App;

fn defaults() -> Defaults {
    Defaults {
        database: "pas",
        port: 5150,
        production_port: None,
        body_limit: 2 * MB,
        max_connections: 10,
        // The PAS database is not this service's: `pas-migrate` applies its migrations.
        auto_migrate: false,
        ..Defaults::default()
    }
}

/// The value of a setting for this service when the environment does not give one, where it
/// differs from axa's. Production has no `DATABASE_URL` here, so it refuses to boot without one.
fn setting(environment: &Environment, name: &str) -> Option<&'static str> {
    let production = environment.is_production_like();
    let test = *environment == Environment::Test;
    match name {
        "DATABASE_URL" if test => Some("postgres://pas:pas_dev_password@localhost:5432/pas_test"),
        "DATABASE_URL" if !production => Some("postgres://pas:pas_dev_password@localhost:5432/pas"),
        "DB_CONNECT_TIMEOUT" => Some(if production { "10000" } else { "5000" }),
        "DB_IDLE_TIMEOUT" => Some(if production { "30000" } else { "5000" }),
        "DB_MAX_CONNECTIONS" if test => Some("2"),
        "HTTP_CSP" => Some(PAGE_CSP),
        "HTTP_COMPRESSION" if !test => Some("true"),
        _ => None,
    }
}

/// A setting from the process environment, or else this service's own value for it.
fn lookup(environment: &Environment, name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| setting(environment, name).map(String::from))
}

impl App {
    /// This service's configuration for `environment`, from its process environment.
    ///
    /// # Errors
    ///
    /// Fails when production has no `DATABASE_URL`, or lists `*` as a CORS origin.
    pub fn config(environment: &Environment) -> Result<Config> {
        Config::from_lookup(environment, &defaults(), |name| lookup(environment, name))
    }

    /// Connects to the database.
    ///
    /// # Errors
    ///
    /// Fails when the database cannot be reached.
    pub async fn context(environment: Environment, config: Config) -> Result<AppContext> {
        let db = axa::db::connect(&config.database).await?;
        Ok(AppContext::new(environment, db, config))
    }

    fn routes() -> AppRoutes {
        // Not `with_default_routes()`: `controllers::health` has its own `/_health`, which also
        // reports the service and its version. `probe_routes` is axa's `/_ping` and `/_readiness`.
        AppRoutes::empty()
            .add_route(controllers::dashboard::routes())
            .add_route(controllers::patients::routes())
            .add_route(controllers::wards::routes())
            .add_route(controllers::rtt::routes())
            .add_route(controllers::admissions::routes())
            .add_route(controllers::appointments::routes())
            .add_route(controllers::letters::routes())
            .add_route(controllers::health::routes())
            .add_route(controllers::health::probe_routes())
    }

    /// The whole service. The static files are inside the middleware, so they get its headers.
    ///
    /// # Errors
    ///
    /// Fails when a template does not load, or a CORS origin is invalid.
    pub fn router(ctx: &AppContext) -> Result<Router> {
        let router = Self::routes()
            .into_router(ctx.clone())
            .nest_service("/static", ServeDir::new(STATIC_FOLDER).precompressed_gzip())
            .layer(Extension(Views::load()?))
            // Inside axa's middleware, which has already cleaned the id or made one (AXA-6).
            .layer(from_fn(axa::tracing::track));
        middleware::apply(router, &ctx.config.http)
    }

    async fn db(ctx: &AppContext, command: DbCommand) -> Result<()> {
        match command {
            DbCommand::Migrate => migration::up(&ctx.db).await.map_err(Error::wrap),
            DbCommand::Status => {
                let migrations = migration::status(&ctx.db).await.map_err(Error::wrap)?;
                for m in migrations {
                    let state = if m.applied { "applied" } else { "pending" };
                    println!("{state:8}{} {}", m.version, m.description);
                }
                Ok(())
            }
            DbCommand::Down { .. } | DbCommand::Reset | DbCommand::Truncate => Err(Error::string(
                "this service shares the PAS database and will not revert or empty it",
            )),
        }
    }

    /// Runs the command line.
    ///
    /// # Errors
    ///
    /// Fails when the service cannot boot, or the command fails.
    pub async fn run(cli: Cli) -> Result<()> {
        let environment = cli
            .environment
            .as_deref()
            .map_or_else(Environment::from_env, Environment::parse);
        // Before the database, so that settings can be checked anywhere they are going to run.
        if matches!(cli.command, Command::Check) {
            let report =
                cli::check_report(&environment, &defaults(), |name| lookup(&environment, name))?;
            print!("{report}");
            return Ok(());
        }
        let mut config = Self::config(&environment)?;
        if let Command::Start { binding, port, .. } = &cli.command {
            if let Some(binding) = binding {
                config.server.binding.clone_from(binding);
            }
            if let Some(port) = port {
                config.server.port = *port;
            }
        }
        let ctx = Self::context(environment, config).await?;
        axa::logger::init(env!("CARGO_CRATE_NAME"), &ctx.config.logger);
        match cli.command {
            Command::Start { .. } => server::serve(Self::router(&ctx)?, &ctx.config.server).await,
            Command::Db { command } => Self::db(&ctx, command).await,
            // There are no tasks and no background jobs: the PAS binary owns the outbox dispatcher.
            Command::Task { name: None, .. } => Ok(()),
            Command::Task { name: Some(_), .. } => Err(Error::string("this service has no tasks")),
            Command::Check => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_no_environment(environment: &Environment) -> Result<Config> {
        Config::from_lookup(environment, &defaults(), |name| {
            setting(environment, name).map(String::from)
        })
    }

    #[test]
    fn production_refuses_to_boot_without_a_database_url() {
        assert!(with_no_environment(&Environment::Production).is_err());
    }

    #[test]
    fn development_and_test_reach_the_pas_database() {
        let development = with_no_environment(&Environment::Development).unwrap();
        assert!(development.database.uri.ends_with("@localhost:5432/pas"));
        let test = with_no_environment(&Environment::Test).unwrap();
        assert!(test.database.uri.ends_with("@localhost:5432/pas_test"));
        assert_eq!(test.database.max_connections, 2);
    }

    #[test]
    fn production_keeps_the_pool_it_had() {
        let settings = |name: &str| match name {
            "DATABASE_URL" => Some("postgres://example/pas?sslmode=require".to_string()),
            other => setting(&Environment::Production, other).map(String::from),
        };
        let config = Config::from_lookup(&Environment::Production, &defaults(), settings).unwrap();
        assert_eq!(config.database.min_connections, 2);
        assert_eq!(config.database.max_connections, 20);
        assert_eq!(config.database.connect_timeout, 10_000);
        assert_eq!(config.database.idle_timeout, 30_000);
        assert_eq!(config.server.port, 5150);
        assert!(config.http.cors_origins.is_empty(), "no CORS in production");
    }

    #[test]
    fn production_refuses_a_database_url_that_does_not_ask_for_tls() {
        let settings = |name: &str| match name {
            "DATABASE_URL" => Some("postgres://example/pas".to_string()),
            other => setting(&Environment::Production, other).map(String::from),
        };
        assert!(Config::from_lookup(&Environment::Production, &defaults(), settings).is_err());
    }

    #[test]
    fn this_service_does_not_migrate_the_database_it_shares() {
        let development = with_no_environment(&Environment::Development).unwrap();
        assert!(!development.database.auto_migrate);
    }

    #[test]
    fn every_environment_sends_the_page_policy() {
        for environment in [Environment::Development, Environment::Test] {
            let config = with_no_environment(&environment).unwrap();
            assert_eq!(config.http.csp.as_deref(), Some(PAGE_CSP));
        }
    }

    /// If the layout starts loading something else, this says the policy would block it.
    #[test]
    fn the_page_policy_allows_what_the_layout_loads() {
        let layout = std::fs::read_to_string("assets/views/layouts/main.html").unwrap();
        assert!(layout.contains("<style>") && PAGE_CSP.contains("style-src 'unsafe-inline'"));
        let scripts: Vec<&str> = layout
            .split("src=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        assert_eq!(scripts.len(), 1, "{scripts:?}");
        let origin = scripts[0].split('/').take(3).collect::<Vec<_>>().join("/");
        assert!(
            PAGE_CSP.contains(&format!("script-src {origin}")),
            "{origin}"
        );
    }

    /// The operator's way to see these settings, and it must work without a database.
    #[test]
    fn check_reports_the_page_policy() {
        let report = cli::check_report(&Environment::Development, &defaults(), |name| {
            setting(&Environment::Development, name).map(String::from)
        })
        .unwrap();
        assert!(report.contains("HTTP_CSP"), "{report}");
    }
}
