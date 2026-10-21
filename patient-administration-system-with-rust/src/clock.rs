//! What time it is (QUAL-25).
//!
//! Code that decides something by the time of day (quiet hours, an offer's expiry, a job's "now") asks a
//! [`Clock`] rather than calling `Utc::now()` itself, so a test can fix the time and pass whatever the wall
//! clock says. Production uses [`SystemClock`]; tests use [`FixedClock`].

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};

/// A source of the current time.
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> DateTime<Utc>;
}

/// A clock shared between services.
pub type SharedClock = Arc<dyn Clock>;

/// The real time.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// The real clock, shared.
pub fn system() -> SharedClock {
    Arc::new(SystemClock)
}

/// A clock that reads what it was set to, for tests.
#[derive(Debug)]
pub struct FixedClock(Mutex<DateTime<Utc>>);

impl FixedClock {
    pub fn at(t: DateTime<Utc>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(t)))
    }

    pub fn set(&self, t: DateTime<Utc>) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = t;
    }

    pub fn advance(&self, by: Duration) {
        let mut t = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *t += by;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_fixed_clock_reads_what_it_was_set_to_and_moves_only_when_told() {
        let t = Utc.with_ymd_and_hms(2030, 1, 1, 22, 0, 0).unwrap();
        let c = FixedClock::at(t);
        assert_eq!(c.now(), t);
        c.advance(Duration::hours(12));
        assert_eq!(c.now(), t + Duration::hours(12));
        c.set(t);
        assert_eq!(c.now(), t);
    }

    #[test]
    fn the_system_clock_is_the_real_time() {
        let before = Utc::now();
        let now = system().now();
        assert!(now >= before && now <= Utc::now());
    }
}
