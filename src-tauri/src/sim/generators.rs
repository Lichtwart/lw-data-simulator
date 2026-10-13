//! Value generators (PRD §8): `randomWalk`, `sine`, `dailyProfile`, `counter`.
//!
//! Pure and deterministic: time comes in as a [`Tick`] and randomness from a [`RandomSource`]
//! (use [`SeededRng`] in the session and in tests). Manual movement is handled by the session:
//! while overridden it does not call [`next_value`]; on "resume generator" it continues from
//! `GeneratorState::last` (set it to the manual value to resume smoothly; a `randomWalk` whose
//! `last` is outside its band drifts back by `step` per tick).
//!
//! Counters are monotonic: seed `GeneratorState::last` at session start with [`counter_seed`]
//! (newest stored measurement, snapshot, default) and reject manual / scenario values below the
//! current value with [`GeneratorState::counter_value_problem`].

use std::f64::consts::TAU;

use crate::model::GeneratorConfig;

/// Source of uniform random numbers in `[0, 1)`.
pub trait RandomSource {
    fn next_unit(&mut self) -> f64;
}

/// Small deterministic PRNG (SplitMix64). Not cryptographic; good enough for demo noise.
#[derive(Debug, Clone)]
pub struct SeededRng(u64);

impl SeededRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
}

impl RandomSource for SeededRng {
    fn next_unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // 53 random bits → [0, 1).
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Clock input of one generator step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    /// Unix time in seconds (drives `sine`).
    pub now_secs: f64,
    /// Seconds since the previous step of this item (drives `counter`).
    pub dt_secs: f64,
    /// Local time of day in hours, `0.0..24.0` (drives `dailyProfile`); see [`local_hour`].
    pub local_hour: f64,
}

/// Local hour of day for `unix_secs` at a UTC offset (seconds, e.g. 7200 for CEST).
pub fn local_hour(unix_secs: f64, utc_offset_secs: f64) -> f64 {
    ((unix_secs + utc_offset_secs) / 3600.0).rem_euclid(24.0)
}

/// Local hour of day in the IANA time zone `tz` (DST-safe, computed per tick).
///
/// `dailyProfile` runs in the **device's** time zone (`lw_demo.timeZone`, default
/// `Europe/Berlin`), not the laptop's. Returns `None` for an unknown zone name; callers fall
/// back to [`DEFAULT_TIME_ZONE`].
pub fn local_hour_in_zone(unix_secs: f64, tz: &str) -> Option<f64> {
    use chrono::{DateTime, TimeZone, Timelike};
    let zone: chrono_tz::Tz = tz.parse().ok()?;
    let whole = unix_secs.floor();
    let utc = DateTime::from_timestamp(whole as i64, 0)?;
    let local = zone.from_utc_datetime(&utc.naive_utc());
    let secs = f64::from(local.hour() * 3600 + local.minute() * 60 + local.second());
    Some((secs + (unix_secs - whole)) / 3600.0)
}

/// Zone used when `lw_demo.timeZone` is absent, empty or unknown (firmware default).
pub const DEFAULT_TIME_ZONE: &str = "Europe/Berlin";

/// Per-item generator state carried between ticks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GeneratorState {
    /// Last produced (or manually set) value. For a `counter` this is also the floor: the
    /// value never goes below it.
    pub last: Option<f64>,
}

impl GeneratorState {
    /// State that continues from `value` (e.g. a counter seeded by [`counter_seed`]).
    pub fn seeded(value: f64) -> Self {
        Self {
            last: value.is_finite().then_some(value),
        }
    }

    /// Raise the state to `value` if that is higher than what it holds (never lowers it, so a
    /// late or stale seed cannot send a counter backwards). Returns whether it changed.
    pub fn seed_counter(&mut self, value: f64) -> bool {
        if !value.is_finite() || self.last.is_some_and(|l| l >= value) {
            return false;
        }
        self.last = Some(value);
        true
    }

    /// Whether a manual / scenario `value` may be applied to a counter in this state.
    pub fn accepts_counter_value(&self, value: f64) -> bool {
        self.counter_value_problem(value).is_none()
    }

    /// Why a manual / scenario `value` must not be applied to a counter (`None` = fine): it is
    /// not finite or lower than the current value.
    pub fn counter_value_problem(&self, value: f64) -> Option<String> {
        if !value.is_finite() {
            return Some("counter value must be a finite number".into());
        }
        match self.last {
            Some(current) if value < current => Some(format!(
                "counter value {value} is below the current value {current} (counters never decrease)"
            )),
            _ => None,
        }
    }
}

/// Start value of a counter at session start: the highest of the newest stored measurement
/// (`GET /measurement/measurements?...&revert=true&pageSize=1`), the `lw_demo.snapshot` value
/// and the default (control default / `initCount`). Taking the maximum means a restart without
/// "Save snapshot" never sends the counter backwards.
pub fn counter_seed(default: f64, snapshot: Option<f64>, newest_remote: Option<f64>) -> f64 {
    [Some(default), snapshot, newest_remote]
        .into_iter()
        .flatten()
        .filter(|v| v.is_finite())
        .fold(0.0_f64, f64::max)
}

/// Default `randomWalk` step as a fraction of `spread` when `step` is not set.
pub const DEFAULT_STEP_FRACTION: f64 = 0.2;

/// Next value of `cfg` and update `state`.
///
/// `base` is the item's start / fallback value (control default): the `sine` center when not
/// configured, the `counter` start and the `randomWalk` start when `state` is empty.
/// - `randomWalk`: last ± uniform(step), reflected into `[center − spread, center + spread]`;
///   starts at the center. A `last` outside the band (manual value) moves by `step` per tick
///   towards the band and does not reflect until it is inside.
/// - `sine`: `center + amplitude · sin(2π · now / period)`.
/// - `dailyProfile`: linear interpolation between the 24 hourly points (23:00 → 00:00 wraps);
///   fewer than 24 points are padded with the last one (validation rejects that in libraries).
/// - `counter`: monotonic, `last + ratePerHour · dt / 3600` (never decreases).
pub fn next_value(
    cfg: &GeneratorConfig,
    state: &mut GeneratorState,
    tick: &Tick,
    base: f64,
    rng: &mut dyn RandomSource,
) -> f64 {
    let value = match cfg {
        GeneratorConfig::RandomWalk {
            center,
            spread,
            step,
        } => {
            let spread = spread.abs();
            let step = step.unwrap_or(spread * DEFAULT_STEP_FRACTION).abs();
            let last = state.last.unwrap_or(*center);
            let (lo, hi) = (center - spread, center + spread);
            if last > hi {
                // Mean reversion: drift back instead of folding into an arbitrary point.
                (last - step).max(hi)
            } else if last < lo {
                (last + step).min(lo)
            } else {
                let delta = (rng.next_unit() * 2.0 - 1.0) * step;
                reflect(last + delta, lo, hi)
            }
        }
        GeneratorConfig::Sine {
            period,
            amplitude,
            center,
        } => {
            let center = center.unwrap_or(base);
            if *period > 0.0 {
                center + amplitude * (TAU * tick.now_secs / period).sin()
            } else {
                center
            }
        }
        GeneratorConfig::DailyProfile { points } => daily_profile(points, tick.local_hour, base),
        GeneratorConfig::Counter { rate_per_hour } => {
            let last = state.last.unwrap_or(base);
            (last + rate_per_hour.max(0.0) * tick.dt_secs.max(0.0) / 3600.0).max(last)
        }
    };
    state.last = Some(value);
    value
}

/// Reflect `v` back into `[lo, hi]` (a walk bounces off the edges instead of sticking to them).
fn reflect(v: f64, lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        return lo;
    }
    let mut v = v;
    // At most a couple of iterations because |step| is small; bounded for safety.
    for _ in 0..8 {
        if v > hi {
            v = 2.0 * hi - v;
        } else if v < lo {
            v = 2.0 * lo - v;
        } else {
            return v;
        }
    }
    v.clamp(lo, hi)
}

fn daily_profile(points: &[f64], hour: f64, fallback: f64) -> f64 {
    let Some(&last) = points.last() else {
        return fallback;
    };
    let at = |i: usize| points.get(i % 24).copied().unwrap_or(last);
    let h = hour.rem_euclid(24.0);
    let i = h.floor() as usize; // 0..=23
    let frac = h - h.floor();
    at(i) + (at(i + 1) - at(i)) * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(now_secs: f64, dt_secs: f64, local_hour: f64) -> Tick {
        Tick {
            now_secs,
            dt_secs,
            local_hour,
        }
    }

    /// Always returns the same number.
    struct Fixed(f64);
    impl RandomSource for Fixed {
        fn next_unit(&mut self) -> f64 {
            self.0
        }
    }

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = SeededRng::new(42);
        let mut b = SeededRng::new(42);
        for _ in 0..1000 {
            let x = a.next_unit();
            assert_eq!(x, b.next_unit());
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(SeededRng::new(1).next_unit(), SeededRng::new(2).next_unit());
    }

    #[test]
    fn random_walk_stays_in_band_and_respects_step() {
        let cfg = GeneratorConfig::RandomWalk {
            center: -18.0,
            spread: 1.5,
            step: Some(0.4),
        };
        let mut st = GeneratorState::default();
        let mut rng = SeededRng::new(7);
        let mut prev = -18.0;
        let mut distinct = std::collections::BTreeSet::new();
        for i in 0..5000 {
            let v = next_value(&cfg, &mut st, &tick(f64::from(i), 1.0, 0.0), 0.0, &mut rng);
            assert!((-19.5..=-16.5).contains(&v), "{v}");
            assert!((v - prev).abs() <= 0.4 + 1e-9, "{prev} → {v}");
            prev = v;
            distinct.insert((v * 1000.0) as i64);
        }
        assert!(distinct.len() > 100);
        // Same seed → same sequence.
        let run = |seed| {
            let mut st = GeneratorState::default();
            let mut rng = SeededRng::new(seed);
            (0..10)
                .map(|_| next_value(&cfg, &mut st, &tick(0.0, 1.0, 0.0), 0.0, &mut rng))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(3), run(3));
    }

    #[test]
    fn random_walk_reflects_at_edges_and_defaults_step() {
        let cfg = GeneratorConfig::RandomWalk {
            center: 0.0,
            spread: 1.0,
            step: None,
        };
        let mut st = GeneratorState { last: Some(0.9) };
        // rng 1.0-ε → +step (0.2): 1.1 reflects to 0.9.
        let v = next_value(
            &cfg,
            &mut st,
            &tick(0.0, 1.0, 0.0),
            0.0,
            &mut Fixed(0.999_999_999),
        );
        assert!((v - 0.9).abs() < 1e-6, "{v}");
        // Starts at the center when there is no last value; rng 0.5 → no change.
        let mut st = GeneratorState::default();
        assert_eq!(
            next_value(&cfg, &mut st, &tick(0.0, 1.0, 0.0), 5.0, &mut Fixed(0.5)),
            0.0
        );
    }

    #[test]
    fn sine_follows_period_and_amplitude() {
        let cfg = GeneratorConfig::Sine {
            period: 60.0,
            amplitude: 2.0,
            center: Some(10.0),
        };
        let mut st = GeneratorState::default();
        let mut rng = Fixed(0.0);
        let at = |t: f64, st: &mut GeneratorState, rng: &mut Fixed| {
            next_value(&cfg, st, &tick(t, 1.0, 0.0), 0.0, rng)
        };
        assert!((at(0.0, &mut st, &mut rng) - 10.0).abs() < 1e-9);
        assert!((at(15.0, &mut st, &mut rng) - 12.0).abs() < 1e-9);
        assert!((at(45.0, &mut st, &mut rng) - 8.0).abs() < 1e-9);
        assert!((at(75.0, &mut st, &mut rng) - 12.0).abs() < 1e-9);
        // Center defaults to `base`.
        let cfg = GeneratorConfig::Sine {
            period: 60.0,
            amplitude: 2.0,
            center: None,
        };
        let v = next_value(&cfg, &mut st, &tick(15.0, 1.0, 0.0), 20.0, &mut rng);
        assert!((v - 22.0).abs() < 1e-9);
    }

    #[test]
    fn daily_profile_interpolates_and_wraps() {
        let points: Vec<f64> = (0..24).map(|h| f64::from(h) * 10.0).collect();
        let cfg = GeneratorConfig::DailyProfile { points };
        let mut st = GeneratorState::default();
        let mut rng = Fixed(0.0);
        let mut at = |h: f64| next_value(&cfg, &mut st, &tick(0.0, 1.0, h), 0.0, &mut rng);
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(6.0), 60.0);
        assert!((at(6.5) - 65.0).abs() < 1e-9);
        // 23:30 → halfway between 230 and the 00:00 point (0).
        assert!((at(23.5) - 115.0).abs() < 1e-9);
        assert_eq!(at(24.0), 0.0);
        assert_eq!(local_hour(0.0, 7200.0), 2.0);
        assert_eq!(local_hour(86_400.0 * 3.0 + 3600.0 * 13.5, 0.0), 13.5);
    }

    #[test]
    fn counter_is_monotonic_with_rate_per_hour() {
        let cfg = GeneratorConfig::Counter {
            rate_per_hour: 12.0,
        };
        let mut st = GeneratorState::default();
        let mut rng = Fixed(0.0);
        let v = next_value(&cfg, &mut st, &tick(0.0, 1800.0, 0.0), 1000.0, &mut rng);
        assert!((v - 1006.0).abs() < 1e-9);
        let v2 = next_value(&cfg, &mut st, &tick(0.0, 3600.0, 0.0), 1000.0, &mut rng);
        assert!((v2 - 1018.0).abs() < 1e-9);
        // Negative dt / rate never decrease the counter.
        let v3 = next_value(&cfg, &mut st, &tick(0.0, -50.0, 0.0), 1000.0, &mut rng);
        assert_eq!(v3, v2);
        // Resume after a manual value continues from it.
        st.last = Some(5000.0);
        let v4 = next_value(&cfg, &mut st, &tick(0.0, 300.0, 0.0), 1000.0, &mut rng);
        assert!((v4 - 5001.0).abs() < 1e-9);
    }

    #[test]
    fn random_walk_outside_band_drifts_back_by_step() {
        let cfg = GeneratorConfig::RandomWalk {
            center: -18.0,
            spread: 1.5,
            step: Some(0.5),
        };
        // Manual value 0 on a -18 ± 1.5 band: no jump, one step per tick towards the band.
        let mut st = GeneratorState { last: Some(0.0) };
        let mut rng = SeededRng::new(1);
        let mut prev = 0.0;
        for _ in 0..60 {
            let v = next_value(&cfg, &mut st, &tick(0.0, 1.0, 0.0), 0.0, &mut rng);
            assert!(prev - v <= 0.5 + 1e-9, "{prev} → {v}");
            prev = v;
        }
        assert!((-19.5..=-16.5).contains(&prev), "{prev}");
        // Below the band it drifts up and lands exactly on the edge.
        let mut st = GeneratorState { last: Some(-30.0) };
        let mut v = -30.0;
        for _ in 0..40 {
            v = next_value(&cfg, &mut st, &tick(0.0, 1.0, 0.0), 0.0, &mut Fixed(0.5));
            assert!(v <= -19.5 || (-19.5..=-16.5).contains(&v));
        }
        assert!((-19.5..=-16.5).contains(&v), "{v}");
    }

    #[test]
    fn counter_seed_takes_the_highest_source() {
        assert_eq!(counter_seed(1000.0, None, None), 1000.0);
        assert_eq!(counter_seed(1000.0, Some(1500.0), None), 1500.0);
        assert_eq!(counter_seed(1000.0, Some(1500.0), Some(1800.0)), 1800.0);
        // A stale remote value below the default never lowers the start.
        assert_eq!(counter_seed(1000.0, None, Some(10.0)), 1000.0);
        assert_eq!(counter_seed(1000.0, Some(f64::NAN), None), 1000.0);
    }

    #[test]
    fn counter_state_rejects_lower_values_and_seed_never_lowers() {
        let mut st = GeneratorState::default();
        assert!(st.accepts_counter_value(0.0));
        assert!(st.seed_counter(5000.0));
        assert!(!st.seed_counter(100.0));
        assert_eq!(st.last, Some(5000.0));
        assert!(st.counter_value_problem(4999.0).is_some());
        assert!(st.accepts_counter_value(5000.0));
        assert!(st.accepts_counter_value(6000.0));
        assert!(st.counter_value_problem(f64::NAN).is_some());
    }

    /// Resume after an app restart: seeding from the newest stored value means the counter
    /// never goes below what Cumulocity already has.
    #[test]
    fn resume_after_restart_never_decreases() {
        let cfg = GeneratorConfig::Counter {
            rate_per_hour: 60.0,
        };
        let mut rng = Fixed(0.0);
        let mut st = GeneratorState::seeded(counter_seed(1000.0, None, None));
        let mut sent = 0.0;
        for _ in 0..10 {
            sent = next_value(&cfg, &mut st, &tick(0.0, 60.0, 0.0), 1000.0, &mut rng);
        }
        assert!((sent - 1010.0).abs() < 1e-9);
        // "Restart": a fresh state seeded from the newest remote value.
        let mut st = GeneratorState::seeded(counter_seed(1000.0, None, Some(sent)));
        let next = next_value(&cfg, &mut st, &tick(0.0, 60.0, 0.0), 1000.0, &mut rng);
        assert!(next >= sent, "{next} < {sent}");
        // A snapshot older than the remote value cannot pull it back.
        let mut st = GeneratorState::seeded(counter_seed(1000.0, Some(1003.0), Some(sent)));
        assert!(next_value(&cfg, &mut st, &tick(0.0, 0.0, 0.0), 1000.0, &mut rng) >= sent);
    }

    #[test]
    fn zone_aware_local_hour_follows_dst() {
        // 2026-07-01 10:00 UTC = 12:00 CEST; 2026-01-01 10:00 UTC = 11:00 CET.
        let summer = 1_782_900_000.0; // 2026-07-01T10:00:00Z
        let winter = 1_767_261_600.0; // 2026-01-01T10:00:00Z
        assert!((local_hour_in_zone(summer, "Europe/Berlin").unwrap() - 12.0).abs() < 1e-9);
        assert!((local_hour_in_zone(winter, "Europe/Berlin").unwrap() - 11.0).abs() < 1e-9);
        assert!((local_hour_in_zone(summer, "UTC").unwrap() - 10.0).abs() < 1e-9);
        assert!(local_hour_in_zone(summer, "Mars/Base").is_none());
    }
}
