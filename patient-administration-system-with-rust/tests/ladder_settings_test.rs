//! The reminder ladder's settings: built from the environment with its defaults, adjusted by the builders, and
//! asked about an appointment that does not exist. Needs PostgreSQL: `cargo test -- --ignored`.
mod common;
use patient_administration_system::communication::consent::QuietHours;
use patient_administration_system::communication::ladder::{LadderService, parse_steps};
use uuid::Uuid;

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_ladder_is_built_from_the_environment_and_adjusted_by_its_builders() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let from_env = LadderService::from_env(s.db.clone(), s.communication.clone());
    assert!(
        from_env.is_ok(),
        "the defaults are valid: {:?}",
        from_env.err()
    );
    let steps = parse_steps("main=3d,final=1d").expect("two steps");
    let ladder = LadderService::new(s.db.clone(), s.communication.clone())
        .with_steps(steps)
        .with_max_touches(2)
        // A trailing slash is dropped, and a blank base is no base.
        .with_link_base(Some("https://pas.example/r/".into()))
        .with_link_base(Some("  ".into()))
        .with_quiet_hours(Some(QuietHours::parse("21:00-08:00").unwrap()))
        .with_quiet_hours(None);
    // Nobody has acted on an appointment that is not there.
    assert!(!ladder.has_acted(Uuid::new_v4()).await.unwrap());
}
