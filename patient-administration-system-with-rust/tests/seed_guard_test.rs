//! `pas-seed` refuses outside development and test, before it touches a database, as axa's
//! `Task::development_only` does. Needs no database: a run that is allowed stops at the missing `DATABASE_URL`.

use std::process::Command;

fn seed(environment: &str) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_pas-seed"))
        .env("APP_ENV", environment)
        .env_remove("DATABASE_URL")
        .output()
        .expect("pas-seed runs");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn production_like_environments_are_refused() {
    for environment in ["production", "staging", "anything-else"] {
        let (code, stderr) = seed(environment);
        assert_eq!(code, Some(2), "{environment}: {stderr}");
        assert!(stderr.contains("refused"), "{environment}: {stderr}");
        assert!(
            !stderr.contains("DATABASE_URL"),
            "{environment}: refused before the database: {stderr}"
        );
    }
}

#[test]
fn development_and_test_get_past_the_guard() {
    for environment in ["development", "test"] {
        let (code, stderr) = seed(environment);
        assert_eq!(code, Some(2), "{environment}: {stderr}");
        assert!(!stderr.contains("refused"), "{environment}: {stderr}");
        assert!(
            stderr.contains("DATABASE_URL is required"),
            "{environment}: {stderr}"
        );
    }
}
