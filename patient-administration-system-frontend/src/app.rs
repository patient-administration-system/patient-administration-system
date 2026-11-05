//! The application: its configuration, its routes and its command line, as plain functions.
//!
//! This service shares the PAS database and never owns it, so it does not migrate on boot, and
//! `db reset`, `db down` and `db truncate` refuse. Migrations are applied by `pas-migrate`.

use axa::{
    AppContext, Environment, Error, Result,
    cli::{Cli, Command, DbCommand},
    config::{Config, Defaults, MB},
    middleware,
    routes::AppRoutes,
    server,
};
use axum::{Extension, Router};
use migration::{Migrator, MigratorTrait};
use tower_http::services::ServeDir;

use crate::{controllers, views::Views};

/// Served at `/static`. Files may be absent: a service with no static files still boots.
const STATIC_FOLDER: &str = "assets/static";

pub struct App;

fn defaults() -> Defaults {
    Defaults {
        database: "pas",
        port: 5150,
        production_port: None,
        body_limit: 2 * MB,
        max_connections: 10,
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
        "HTTP_COMPRESSION" if !test => Some("true"),
        _ => None,
    }
}

impl App {
    /// This service's configuration for `environment`, from its process environment.
    ///
    /// # Errors
    ///
    /// Fails when production has no `DATABASE_URL`, or lists `*` as a CORS origin.
    pub fn config(environment: &Environment) -> Result<Config> {
        Config::from_lookup(environment, &defaults(), |name| {
            std::env::var(name)
                .ok()
                .or_else(|| setting(environment, name).map(String::from))
        })
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
        // reports the database.
        AppRoutes::empty()
            .add_route(controllers::dashboard::routes())
            .add_route(controllers::patients::routes())
            .add_route(controllers::wards::routes())
            .add_route(controllers::rtt::routes())
            .add_route(controllers::admissions::routes())
            .add_route(controllers::appointments::routes())
            .add_route(controllers::letters::routes())
            .add_route(controllers::health::routes())
            .add_route(controllers::health::ping_routes())
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
            .layer(Extension(Views::load()?));
        middleware::apply(router, &ctx.config.http)
    }

    async fn db(ctx: &AppContext, command: DbCommand) -> Result<()> {
        match command {
            DbCommand::Migrate => Ok(Migrator::up(&ctx.db, None).await?),
            DbCommand::Status => Ok(Migrator::status(&ctx.db).await?),
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
            "DATABASE_URL" => Some("postgres://example/pas".to_string()),
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
}
