//! Send scheduling (PRD §8): immediate send on change + periodic full send.
//!
//! - **Immediate**: scenario / operation / group / fault changes are sent at the end of the loop
//!   iteration that made them (batched into `measurementCollection`s by
//!   `payload::measurement`). **Manual** changes (a slider drag fires many per second) are
//!   coalesced: sent [`SEND_DEBOUNCE`] after the first one, with the latest values.
//! - **Periodic**: every `sendIntervalSec` (5–899 s; the floorplan plugin marks values stale
//!   after `measurementIntervalMinutes` + 5 min = 900 s) all values are sent.
//! - **Generators** step every [`GENERATOR_TICK`] for a live UI (`item-values-changed`); their
//!   values reach Cumulocity with the periodic send, so a 5 s random walk does not flood the
//!   tenant.
//!
//! [`Scheduler`] is pure (it takes `Instant`s), so the timing is unit-tested without a runtime.

use std::time::Duration;

use tokio::time::Instant;

/// Minimum allowed send interval in seconds (library schema: 5–899). Below 5 s a gas-station
/// session would make more than 60 requests per minute against a shared tenant.
pub const MIN_SEND_INTERVAL_SEC: u32 = 5;
/// Maximum allowed send interval (stale threshold 900 s − 1).
pub const MAX_SEND_INTERVAL_SEC: u32 = 899;
/// Default send interval when a device has none.
pub const DEFAULT_SEND_INTERVAL_SEC: u32 = 30;
/// Generator step period.
pub const GENERATOR_TICK: Duration = Duration::from_secs(5);
/// Manual changes (slider drags) are sent at most this often: the first change opens a window,
/// every later change inside it only updates the value that goes out when the window closes.
pub const SEND_DEBOUNCE: Duration = Duration::from_millis(300);

/// What is due at a given instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Due {
    pub generators: bool,
    pub periodic: bool,
    pub lock_refresh: bool,
}

/// Periodic timers of a session.
#[derive(Debug, Clone)]
pub struct Scheduler {
    interval: Duration,
    lock_interval: Duration,
    next_periodic: Instant,
    next_generators: Instant,
    next_lock: Instant,
}

impl Scheduler {
    /// Timers starting at `now`: the first periodic send is one interval away (the session sends
    /// everything once right at the start).
    pub fn new(now: Instant, interval_sec: u32, lock_interval: Duration) -> Self {
        let interval = Duration::from_secs(u64::from(interval_sec.max(MIN_SEND_INTERVAL_SEC)));
        Self {
            interval,
            lock_interval,
            next_periodic: now + interval,
            next_generators: now + GENERATOR_TICK,
            next_lock: now + lock_interval,
        }
    }

    pub fn interval_sec(&self) -> u32 {
        self.interval.as_secs() as u32
    }

    /// Change the interval; the next periodic send is one new interval after `now`.
    pub fn set_interval(&mut self, now: Instant, interval_sec: u32) {
        self.interval = Duration::from_secs(u64::from(interval_sec.max(MIN_SEND_INTERVAL_SEC)));
        self.next_periodic = now + self.interval;
    }

    /// Earliest instant something is due.
    pub fn next_deadline(&self) -> Instant {
        self.next_periodic
            .min(self.next_generators)
            .min(self.next_lock)
    }

    /// What is due at `now`; advances the fired timers (without drift accumulation: a late
    /// wake-up does not fire twice).
    pub fn poll(&mut self, now: Instant) -> Due {
        let mut due = Due::default();
        if now >= self.next_generators {
            due.generators = true;
            self.next_generators = advance(self.next_generators, GENERATOR_TICK, now);
        }
        if now >= self.next_periodic {
            due.periodic = true;
            self.next_periodic = advance(self.next_periodic, self.interval, now);
        }
        if now >= self.next_lock {
            due.lock_refresh = true;
            self.next_lock = advance(self.next_lock, self.lock_interval, now);
        }
        due
    }
}

fn advance(mut at: Instant, step: Duration, now: Instant) -> Instant {
    while at <= now {
        at += step;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_and_generator_timers() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(t0, 30, Duration::from_secs(180));
        assert_eq!(s.next_deadline(), t0 + GENERATOR_TICK);
        assert_eq!(s.poll(t0 + Duration::from_secs(1)), Due::default());
        let d = s.poll(t0 + Duration::from_secs(5));
        assert!(d.generators && !d.periodic);
        let d = s.poll(t0 + Duration::from_secs(30));
        assert!(d.generators && d.periodic && !d.lock_refresh);
        assert_eq!(s.next_deadline(), t0 + Duration::from_secs(35));
        // A late wake-up fires once and re-aligns.
        let d = s.poll(t0 + Duration::from_secs(100));
        assert!(d.periodic);
        assert_eq!(s.next_deadline(), t0 + Duration::from_secs(105));
        let d = s.poll(t0 + Duration::from_secs(180));
        assert!(d.lock_refresh);
    }

    #[test]
    fn interval_change_restarts_period() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(t0, 600, Duration::from_secs(180));
        s.set_interval(t0 + Duration::from_secs(10), 20);
        assert_eq!(s.interval_sec(), 20);
        assert!(!s.poll(t0 + Duration::from_secs(29)).periodic);
        assert!(s.poll(t0 + Duration::from_secs(30)).periodic);
        s.set_interval(t0, 0);
        assert_eq!(s.interval_sec(), MIN_SEND_INTERVAL_SEC);
    }
}
