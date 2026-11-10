//! `pas-migrate` — applies the PAS migrations.
//!
//! Reads `DATABASE_URL` from the environment (or a `.env` file via `dotenvy`):
//!
//! - `up`     — apply all pending migrations (default)
//! - `down`   — revert every migration that can be reverted (the first six are forward-only)
//! - `fresh`  — drop everything and re-apply; this destroys all data
//! - `status` — print each migration and whether it has run
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(not(test), forbid(unsafe_code))]
use axa_postgres_sqlx::{PoolConfig, connect};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let pool = connect(&PoolConfig::new(url)).await?;
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "up".to_string());
    match cmd.as_str() {
        "up" => migration::up(&pool).await?,
        "down" => migration::down(&pool, u32::MAX).await?,
        "fresh" => axa_postgres_sqlx::dangerously_recreate(&pool, &migration::MIGRATOR).await?,
        "status" => {
            for m in migration::status(&pool).await? {
                let state = if m.applied { "applied" } else { "pending" };
                println!("{state:8}{} {}", m.version, m.description);
            }
        }
        other => return Err(format!("unknown command: {other}").into()),
    }
    println!("ok");
    Ok(())
}
