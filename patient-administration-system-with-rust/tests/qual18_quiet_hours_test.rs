//! QUAL-18: the gaps mutation testing found in the quiet-hours rule (`communication::consent`). No database.
//!
//! `cargo mutants` showed that turning `<` into `<=` at the end of the window, and any change to how
//! `PAS_QUIET_HOURS` is read, passed every test. These pin both down. Run single-threaded: one test sets an
//! environment variable.

use chrono::{TimeZone, Utc};
use chrono_tz::Europe::London;
use patient_administration_system::communication::consent::{DEFAULT_QUIET_HOURS, QuietHours};

fn london(h: u32, m: u32) -> chrono::DateTime<Utc> {
    // A winter date, so London is UTC and the hours read plainly.
    London
        .with_ymd_and_hms(2030, 1, 15, h, m, 0)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn the_window_starts_on_its_first_minute_and_ends_before_its_last() {
    let q = QuietHours::parse("21:00-08:00").unwrap();
    assert!(!q.contains(london(20, 59), London));
    assert!(q.contains(london(21, 0), London), "21:00 is already quiet");
    assert!(q.contains(london(0, 0), London));
    assert!(q.contains(london(7, 59), London));
    assert!(
        !q.contains(london(8, 0), London),
        "08:00 is no longer quiet"
    );

    let day = QuietHours::parse("13:00-14:00").unwrap();
    assert!(day.contains(london(13, 0), London));
    assert!(
        !day.contains(london(14, 0), London),
        "14:00 is no longer quiet"
    );
    assert!(!day.contains(london(12, 59), London));
}

#[test]
fn the_setting_is_read_as_written_off_or_the_default() {
    let read = |v: Option<&str>| {
        // SAFETY: this binary's tests run one at a time where they set the environment.
        unsafe {
            match v {
                Some(v) => std::env::set_var("PAS_QUIET_HOURS", v),
                None => std::env::remove_var("PAS_QUIET_HOURS"),
            }
        }
        QuietHours::from_env()
    };
    let default = QuietHours::parse(DEFAULT_QUIET_HOURS).unwrap();
    assert_eq!(
        read(None).unwrap(),
        Some(default),
        "unset means the default"
    );
    assert_eq!(read(Some("off")).unwrap(), None);
    assert_eq!(read(Some(" OFF ")).unwrap(), None);
    assert_eq!(
        read(Some("22:30-07:00")).unwrap(),
        Some(QuietHours::parse("22:30-07:00").unwrap())
    );
    assert!(
        read(Some("nonsense")).is_err(),
        "a set but unreadable value is refused"
    );
    let _ = read(None);
}
