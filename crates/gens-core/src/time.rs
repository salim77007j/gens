//! Time system: clocks with time scaling and deterministic time sources.
//!
//! Two pieces:
//! - [`InstantSource`]: where raw time comes from. Production uses
//!   [`MonotonicSource`]; tests and headless demos use [`FixedSource`], which
//!   advances by an exact step per read so simulations are reproducible.
//! - [`Clock`]: per-frame timing state — raw delta, scaled delta
//!   (`time_scale`), elapsed accumulators, frame counter.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A source of raw (unscaled) monotonic time, measured from process start.
pub trait InstantSource: Send + Sync {
    /// Current time. Must be non-decreasing between calls for a given source.
    fn now(&self) -> Duration;
}

/// Production time source: the OS monotonic clock, measured from construction.
pub struct MonotonicSource {
    start: Instant,
}

impl MonotonicSource {
    /// Starts the clock at zero now.
    pub fn new() -> MonotonicSource {
        MonotonicSource {
            start: Instant::now(),
        }
    }
}

impl Default for MonotonicSource {
    fn default() -> Self {
        Self::new()
    }
}

impl InstantSource for MonotonicSource {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Deterministic time source: every read advances time by exactly `step`.
///
/// Used by unit tests and headless demos (CI) so engine behavior never
/// depends on wall-clock jitter.
pub struct FixedSource {
    step_nanos: u64,
    current_nanos: AtomicU64,
}

impl FixedSource {
    /// Creates a source that advances `step` per read, starting at zero.
    pub fn new(step: Duration) -> FixedSource {
        FixedSource {
            step_nanos: step.as_nanos() as u64,
            current_nanos: AtomicU64::new(0),
        }
    }

    /// Rewinds the source to zero without changing the step.
    pub fn reset(&self) {
        self.current_nanos.store(0, Ordering::SeqCst);
    }
}

impl InstantSource for FixedSource {
    fn now(&self) -> Duration {
        // fetch_add returns the PREVIOUS value, so the first read is t=0
        // and subsequent reads step by exactly `step`.
        let prev = self
            .current_nanos
            .fetch_add(self.step_nanos, Ordering::SeqCst);
        Duration::from_nanos(prev)
    }
}

/// Per-frame timing state for the engine.
///
/// `tick` is called once per frame with the raw delta; everything else is
/// read-only state consumed by systems.
#[derive(Debug, Clone)]
pub struct Clock {
    time_scale: f32,
    raw_delta: Duration,
    scaled_delta: f32,
    elapsed_unscaled: f64,
    elapsed_scaled: f64,
    frame: u64,
}

impl Clock {
    /// A clock at time zero with scale 1.
    pub fn new() -> Clock {
        Clock {
            time_scale: 1.0,
            raw_delta: Duration::ZERO,
            scaled_delta: 0.0,
            elapsed_unscaled: 0.0,
            elapsed_scaled: 0.0,
            frame: 0,
        }
    }

    /// Advances one frame. Negative-scale requests are clamped to 0
    /// (reversed time is not supported); a zero raw delta is legal.
    pub fn tick(&mut self, raw: Duration) {
        self.raw_delta = raw;
        self.scaled_delta = (raw.as_secs_f32() * self.time_scale).max(0.0);
        self.elapsed_unscaled += raw.as_secs_f64();
        self.elapsed_scaled += self.scaled_delta as f64;
        self.frame += 1;
    }

    /// Last raw (unscaled) frame delta.
    pub fn raw_delta(&self) -> Duration {
        self.raw_delta
    }

    /// Last scaled frame delta in seconds (`raw * time_scale`).
    pub fn delta_secs(&self) -> f32 {
        self.scaled_delta
    }

    /// Total unscaled time since the clock started.
    pub fn elapsed_unscaled(&self) -> f64 {
        self.elapsed_unscaled
    }

    /// Total scaled time since the clock started.
    pub fn elapsed_scaled(&self) -> f64 {
        self.elapsed_scaled
    }

    /// Number of `tick` calls so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Current time scale (0 = paused, 1 = real time, 2 = double speed).
    pub fn time_scale(&self) -> f32 {
        self.time_scale
    }

    /// Sets the time scale; negative values are clamped to 0.
    pub fn set_time_scale(&mut self, scale: f32) {
        self.time_scale = scale.max(0.0);
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_source_is_exact_and_monotonic() {
        let src = FixedSource::new(Duration::from_millis(10));
        assert_eq!(src.now(), Duration::ZERO); // first read is t=0
        assert_eq!(src.now(), Duration::from_millis(10));
        assert_eq!(src.now(), Duration::from_millis(20));
        src.reset();
        assert_eq!(src.now(), Duration::ZERO);
    }

    #[test]
    fn clock_counts_frames_and_elapsed() {
        let mut clock = Clock::new();
        for _ in 0..10 {
            clock.tick(Duration::from_millis(100));
        }
        assert_eq!(clock.frame(), 10);
        assert_eq!(clock.raw_delta(), Duration::from_millis(100));
        assert!((clock.elapsed_unscaled() - 1.0).abs() < 1e-9);
        assert!((clock.elapsed_scaled() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn time_scale_multiplies_delta() {
        let mut clock = Clock::new();
        clock.set_time_scale(2.0);
        clock.tick(Duration::from_millis(100));
        assert!((clock.delta_secs() - 0.2).abs() < 1e-6);
        clock.set_time_scale(0.5);
        clock.tick(Duration::from_millis(100));
        assert!((clock.delta_secs() - 0.05).abs() < 1e-6);
        assert!((clock.elapsed_unscaled() - 0.2).abs() < 1e-9); // raw keeps counting
        assert!((clock.elapsed_scaled() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn zero_scale_pauses_game_time_but_not_frame_count() {
        let mut clock = Clock::new();
        clock.set_time_scale(0.0);
        for _ in 0..5 {
            clock.tick(Duration::from_millis(16));
        }
        assert_eq!(clock.delta_secs(), 0.0);
        assert_eq!(clock.elapsed_scaled(), 0.0);
        assert_eq!(clock.frame(), 5);
        assert!((clock.elapsed_unscaled() - 0.08).abs() < 1e-9);
    }

    #[test]
    fn negative_scale_clamps_to_zero() {
        let mut clock = Clock::new();
        clock.set_time_scale(-5.0);
        assert_eq!(clock.time_scale(), 0.0);
        clock.tick(Duration::from_millis(10));
        assert_eq!(clock.delta_secs(), 0.0);
    }
}
