//! `pas-migrate` — applies the PAS migrations.
//!
//! Reads `DATABASE_URL` from the environment (or a `.env` file via `dotenvy`):
//!
//! - `up`     — apply all pending migrations (default)
//! - `down`   — revert every migration that can be reverted (the first six are forward-only)
//! - `fresh`  — drop everything and re-apply; this destroys all data
//! - `status` — print each migration and whether it has run
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
        clippy::unwrap_in_result
    )
)]
#![cfg_attr(not(test), forbid(unsafe_code))]
use axa_postgres_sqlx::{PoolConfig, connect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Up,
    Down,
    Fresh,
    Status,
}

fn command(value: Option<&str>) -> Result<Command, String> {
    match value.unwrap_or("up") {
        "up" => Ok(Command::Up),
        "down" => Ok(Command::Down),
        "fresh" => Ok(Command::Fresh),
        "status" => Ok(Command::Status),
        other => Err(format!("unknown command: {other}")),
    }
}

fn status_line(applied: bool, version: i64, description: &str) -> String {
    let state = if applied { "applied" } else { "pending" };
    format!("{state:8}{version} {description}")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let pool = connect(&PoolConfig::new(url)).await?;
    let arg = std::env::args().nth(1);
    match command(arg.as_deref())? {
        Command::Up => migration::up(&pool).await?,
        Command::Down => migration::down(&pool, u32::MAX).await?,
        Command::Fresh => {
            axa_postgres_sqlx::dangerously_recreate(&pool, &migration::MIGRATOR).await?;
        }
        Command::Status => {
            for m in migration::status(&pool).await? {
                println!("{}", status_line(m.applied, m.version, &m.description));
            }
        }
    }
    println!("ok");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_and_the_default_are_parsed() {
        assert_eq!(command(None), Ok(Command::Up));
        assert_eq!(command(Some("up")), Ok(Command::Up));
        assert_eq!(command(Some("down")), Ok(Command::Down));
        assert_eq!(command(Some("fresh")), Ok(Command::Fresh));
        assert_eq!(command(Some("status")), Ok(Command::Status));
        assert_eq!(command(Some("undo")), Err("unknown command: undo".into()));
    }

    #[test]
    fn status_lines_align_both_states() {
        assert_eq!(status_line(true, 12, "add slots"), "applied 12 add slots");
        assert_eq!(status_line(false, 13, "add beds"), "pending 13 add beds");
    }
}
