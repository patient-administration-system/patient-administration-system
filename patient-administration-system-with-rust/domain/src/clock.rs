//! What time it is (QUAL-25).
//!
//! Code that decides something by the time of day (quiet hours, an offer's expiry, a job's "now") asks a
//! [`Clock`] rather than calling `Timestamp::now()` itself, so a test can fix the time and pass whatever the wall
//! clock says. Production uses [`SystemClock`]; tests use [`FixedClock`].

use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};

/// A source of the current time.
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> Timestamp;
}

/// A clock shared between services.
pub type SharedClock = Arc<dyn Clock>;

/// The real time.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// The real clock, shared.
pub fn system() -> SharedClock {
    Arc::new(SystemClock)
}

/// A clock that reads what it was set to, for tests.
#[derive(Debug)]
pub struct FixedClock(Mutex<Timestamp>);

impl FixedClock {
    pub fn at(t: Timestamp) -> Arc<Self> {
        Arc::new(Self(Mutex::new(t)))
    }

    pub fn set(&self, t: Timestamp) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = t;
    }

    pub fn advance(&self, by: SignedDuration) {
        let mut t = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *t += by;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_clock_reads_what_it_was_set_to_and_moves_only_when_told() {
        let t = crate::time::utc(2030, 1, 1, 22, 0, 0);
        let c = FixedClock::at(t);
        assert_eq!(c.now(), t);
        c.advance(SignedDuration::from_hours(12));
        assert_eq!(c.now(), t + SignedDuration::from_hours(12));
        c.set(t);
        assert_eq!(c.now(), t);
    }

    #[test]
    fn the_system_clock_is_the_real_time() {
        let before = Timestamp::now();
        let now = system().now();
        assert!(now >= before && now <= Timestamp::now());
    }
}
