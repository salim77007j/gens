//! Fixed-timestep game loop (Phase 1).
//!
//! Contract: simulation updates run at a fixed rate ([`LoopConfig::fixed_dt_secs`])
//! for determinism and replay friendliness; rendering happens once per frame
//! with an interpolation fraction `alpha` telling the renderer how far between
//! the last two simulation states to display.
//!
//! Catch-up policy: after a long hitch, at most [`LoopConfig::max_catchup_steps`]
//! simulation steps run per frame. If the backlog still exceeds one step, the
//! remainder is **dropped** and reported via [`LoopOutcome::backlog_dropped`] —
//! a slow machine falls behind in game-time instead of entering the
//! spiral-of-death where each frame owes more than the last.
//!
//! Simulation steps are driven by *scaled* game time: `time_scale = 0` pauses
//! the simulation while frames and rendering continue (documented engine
//! decision, tested).

use crate::time::{Clock, InstantSource};
use std::time::Duration;

/// Loop tuning parameters.
#[derive(Debug, Clone)]
pub struct LoopConfig {
    /// Simulation step in seconds of *scaled* game time (default 1/60).
    pub fixed_dt_secs: f64,
    /// Maximum simulation steps executed in a single frame (default 5).
    pub max_catchup_steps: u32,
}

impl Default for LoopConfig {
    fn default() -> Self {
        LoopConfig {
            fixed_dt_secs: 1.0 / 60.0,
            max_catchup_steps: 5,
        }
    }
}

/// Result of one `advance` call (pure stepping math, unit-testable without clocks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopOutcome {
    /// Simulation steps that should run this frame.
    pub fixed_steps: u32,
    /// Fraction of a step (0..1) remaining in the accumulator; the renderer
    /// interpolates between the previous and current simulation state by it.
    pub alpha: f32,
    /// True when a hitch forced backlog to be dropped (see module docs).
    pub backlog_dropped: bool,
}

/// Pure accumulator logic of the loop — no clocks, no I/O, fully deterministic.
pub fn advance(config: &LoopConfig, accumulator: &mut f64, scaled_delta: f64) -> LoopOutcome {
    *accumulator += scaled_delta.max(0.0);
    let fixed = config.fixed_dt_secs.max(f64::MIN_POSITIVE);
    let mut steps = 0u32;
    while *accumulator >= fixed && steps < config.max_catchup_steps {
        *accumulator -= fixed;
        steps += 1;
    }
    let mut dropped = false;
    if *accumulator >= fixed {
        dropped = true;
        *accumulator = 0.0;
    }
    LoopOutcome {
        fixed_steps: steps,
        alpha: (*accumulator / fixed).clamp(0.0, 1.0) as f32,
        backlog_dropped: dropped,
    }
}

/// What happened in one [`GameLoop::run_frame`] call.
#[derive(Debug, Clone, Copy)]
pub struct FrameReport {
    /// Simulation steps executed.
    pub fixed_steps: u32,
    /// Render interpolation fraction (0..1).
    pub alpha: f32,
    /// True when backlog was dropped this frame.
    pub backlog_dropped: bool,
    /// Raw unscaled frame delta (wall time).
    pub unscaled_delta: Duration,
}

/// Owns the loop state (accumulator, time source, clock) and executes frames.
pub struct GameLoop<S: InstantSource> {
    config: LoopConfig,
    source: S,
    last: Duration,
    accumulator: f64,
    /// The engine clock, ticked once per frame.
    pub clock: Clock,
}

impl<S: InstantSource> GameLoop<S> {
    /// Builds a loop over the given config and time source.
    pub fn new(config: LoopConfig, source: S) -> GameLoop<S> {
        let last = source.now();
        GameLoop {
            config,
            source,
            last,
            accumulator: 0.0,
            clock: Clock::new(),
        }
    }

    /// Runs one frame: reads raw time, ticks the clock, executes
    /// `fixed_update` once per simulation step (receives the 0-based step
    /// index), and returns the frame report.
    pub fn run_frame<F: FnMut(u32)>(&mut self, mut fixed_update: F) -> FrameReport {
        let now = self.source.now();
        let raw = now.saturating_sub(self.last);
        self.last = now;
        self.clock.tick(raw);

        let outcome = advance(
            &self.config,
            &mut self.accumulator,
            self.clock.delta_secs() as f64,
        );
        for step in 0..outcome.fixed_steps {
            fixed_update(step);
        }

        FrameReport {
            fixed_steps: outcome.fixed_steps,
            alpha: outcome.alpha,
            backlog_dropped: outcome.backlog_dropped,
            unscaled_delta: raw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::FixedSource;
    use std::time::Duration;

    /// Duration must round UP to 1/60 so one raw step always crosses the
    /// fixed threshold (from_nanos truncates 16.666us down and the
    /// accumulator would never fire on frame 1). Not a `const` because
    /// `from_secs_f64` is not const-callable.
    fn step() -> Duration {
        Duration::from_secs_f64(1.0 / 60.0)
    }

    #[test]
    fn one_step_per_exact_delta() {
        let config = LoopConfig::default();
        let mut acc = 0.0;
        for _ in 0..10 {
            let out = advance(&config, &mut acc, 1.0 / 60.0);
            assert_eq!(out.fixed_steps, 1);
            assert!(!out.backlog_dropped);
            assert!(out.alpha.abs() < 1e-6);
        }
    }

    #[test]
    fn multiple_steps_on_large_delta() {
        let config = LoopConfig::default();
        let mut acc = 0.0;
        let out = advance(&config, &mut acc, 3.0 / 60.0);
        assert_eq!(out.fixed_steps, 3);
        assert!(!out.backlog_dropped);
    }

    #[test]
    fn hitch_drops_backlog_instead_of_spiraling() {
        let config = LoopConfig::default(); // max 5 steps
        let mut acc = 0.0;
        // A 10/60-second hitch: 5 steps run, 5/60 of backlog remains.
        let out = advance(&config, &mut acc, 10.0 / 60.0);
        assert_eq!(out.fixed_steps, 5);
        assert!(out.backlog_dropped);
        assert_eq!(acc, 0.0);
        // The next frame is healthy again: exactly one step.
        let out = advance(&config, &mut acc, 1.0 / 60.0);
        assert_eq!(out.fixed_steps, 1);
        assert!(!out.backlog_dropped);
    }

    #[test]
    fn alpha_reflects_partial_step() {
        let config = LoopConfig::default();
        let mut acc = 0.0;
        // Half a step of progress: no step runs, render interpolates at 0.5.
        let out = advance(&config, &mut acc, 1.0 / 120.0);
        assert_eq!(out.fixed_steps, 0);
        assert!((out.alpha - 0.5).abs() < 1e-6);
    }

    #[test]
    fn negative_delta_is_ignored() {
        let config = LoopConfig::default();
        let mut acc = 0.5 / 60.0;
        let out = advance(&config, &mut acc, -10.0);
        assert_eq!(out.fixed_steps, 0);
        assert!((out.alpha - 0.5).abs() < 1e-6); // accumulator unchanged
    }

    #[test]
    fn run_frame_drives_clock_and_closure() {
        let source = FixedSource::new(step());
        let mut game_loop = GameLoop::new(LoopConfig::default(), source);
        let mut sim_time_steps = 0u32;
        for _ in 0..10 {
            let report = game_loop.run_frame(|_| sim_time_steps += 1);
            assert_eq!(report.fixed_steps, 1);
        }
        assert_eq!(sim_time_steps, 10);
        assert_eq!(game_loop.clock.frame(), 10);
    }

    #[test]
    fn zero_time_scale_pauses_simulation_but_not_frames() {
        let source = FixedSource::new(step());
        let mut game_loop = GameLoop::new(LoopConfig::default(), source);
        game_loop.clock.set_time_scale(0.0);
        let mut steps = 0u32;
        for _ in 0..5 {
            let report = game_loop.run_frame(|_| steps += 1);
            assert_eq!(report.fixed_steps, 0);
            assert!(report.unscaled_delta > Duration::ZERO);
        }
        assert_eq!(steps, 0);
        assert_eq!(game_loop.clock.frame(), 5); // frames keep counting
    }
}
