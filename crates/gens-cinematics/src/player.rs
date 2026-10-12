//! Deterministic playback state machine over a [`Timeline`].
//!
//! The player owns **playback state only**: local time, state, speed and
//! loop mode. It has no clock — `advance(dt)` takes the fixed delta as a
//! parameter and returns the events fired by crossing detection over the
//! sampled interval (ADR-0030). All invariants:
//!
//! - `advance` on a paused/idle/finished player is a documented no-op.
//! - A negative or non-finite `dt` (or a `dt*speed` that overflows to
//!   infinity) is a loud error, never a silent clamp.
//! - Events fire on half-open windows `(from, to]` — an event at a key
//!   time fires exactly once per cycle, in timeline order.
//! - Under [`LoopMode::Loop`], a delta that spans cycle boundaries fires
//!   the tail window, then the head window — and a pathological delta
//!   larger than the whole timeline fires every fully-traversed cycle
//!   exactly once per cycle, deterministically.
//! - Sampling (`sample_all`) uses the current local time: 0 while idle,
//!   the clamped end while finished.

use std::sync::Arc;

use thiserror::Error;

use crate::timeline::{SampledValue, TargetChannel, Timeline, TimelineEvent};

/// Loop behavior at the end of the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// Play once and finish (default).
    #[default]
    Once,
    /// Wrap to zero and keep playing.
    Loop,
}

/// Playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackState {
    /// Not started (or stopped); local time is 0.
    #[default]
    Idle,
    /// Advancing on `advance`.
    Playing,
    /// Held at the current local time.
    Paused,
    /// Reached the end of a [`LoopMode::Once`] timeline.
    Finished,
}

/// Errors from player operations.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum PlayerError {
    /// `advance` requires a finite, non-negative delta.
    #[error("advance requires a finite non-negative delta, got {got}")]
    InvalidDelta {
        /// The offending delta.
        got: f32,
    },
    /// `dt * speed` overflowed to infinity.
    #[error("dt * speed overflowed to infinity")]
    DeltaOverflow,
    /// Speed must be finite and > 0.
    #[error("speed must be finite and > 0, got {got}")]
    InvalidSpeed {
        /// The offending speed.
        got: f32,
    },
    /// Seek targets must lie within the timeline.
    #[error("seek target {got} is outside [0, {duration}]")]
    SeekOutOfRange {
        /// The offending target time.
        got: f32,
        /// The timeline duration.
        duration: f32,
    },
}

/// Deterministic playback over an shared [`Timeline`].
#[derive(Debug, Clone)]
pub struct CinematicPlayer {
    timeline: Arc<Timeline>,
    state: PlaybackState,
    local_time: f32,
    speed: f32,
    loop_mode: LoopMode,
}

impl CinematicPlayer {
    /// Creates an idle player at time 0, speed 1, [`LoopMode::Once`].
    #[must_use]
    pub fn new(timeline: Arc<Timeline>) -> CinematicPlayer {
        CinematicPlayer {
            timeline,
            state: PlaybackState::Idle,
            local_time: 0.0,
            speed: 1.0,
            loop_mode: LoopMode::Once,
        }
    }

    /// The shared timeline.
    #[must_use]
    pub fn timeline(&self) -> &Arc<Timeline> {
        &self.timeline
    }

    /// Current playback state.
    #[must_use]
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Current local time in seconds.
    #[must_use]
    pub fn local_time(&self) -> f32 {
        self.local_time
    }

    /// Current playback speed multiplier.
    #[must_use]
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Current loop mode.
    #[must_use]
    pub fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    /// Sets the speed multiplier.
    ///
    /// # Errors
    /// [`PlayerError::InvalidSpeed`] for non-finite or non-positive
    /// speeds — a zero speed would silently freeze playback.
    pub fn set_speed(&mut self, speed: f32) -> Result<(), PlayerError> {
        if !speed.is_finite() || speed <= 0.0 {
            return Err(PlayerError::InvalidSpeed { got: speed });
        }
        self.speed = speed;
        Ok(())
    }

    /// Sets the loop mode.
    pub fn set_loop_mode(&mut self, loop_mode: LoopMode) {
        self.loop_mode = loop_mode;
    }

    /// Starts or resumes playback. On a finished player this restarts
    /// from zero (an explicit, documented semantic — a director that
    /// wants a finished cinematic again calls `play`, not `seek`).
    pub fn play(&mut self) {
        if self.state == PlaybackState::Finished {
            self.local_time = 0.0;
        }
        self.state = PlaybackState::Playing;
    }

    /// Holds playback at the current local time.
    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
        }
    }

    /// Returns to idle at time 0 (no events fire).
    pub fn stop(&mut self) {
        self.state = PlaybackState::Idle;
        self.local_time = 0.0;
    }

    /// Jumps to `time` without firing events (seek is not playback).
    ///
    /// # Errors
    /// [`PlayerError::SeekOutOfRange`] when `time` is outside
    /// `[0, duration]` or non-finite.
    pub fn seek(&mut self, time: f32) -> Result<(), PlayerError> {
        let duration = self.timeline.duration();
        if !time.is_finite() || time < 0.0 || time > duration {
            return Err(PlayerError::SeekOutOfRange {
                got: time,
                duration,
            });
        }
        self.local_time = time;
        Ok(())
    }

    /// Advances playback by `dt` seconds (scaled by speed) and returns
    /// the events whose times were crossed, in firing order.
    ///
    /// No-op on idle/paused/finished players. Events fire on half-open
    /// windows `(from, to]` per cycle; under [`LoopMode::Loop`] the tail
    /// window of a cycle fires before the head window of the next.
    ///
    /// # Errors
    /// [`PlayerError::InvalidDelta`] for negative/non-finite `dt`,
    /// [`PlayerError::DeltaOverflow`] when `dt * speed` overflows.
    pub fn advance(&mut self, dt: f32) -> Result<Vec<TimelineEvent>, PlayerError> {
        let mut fired = Vec::new();
        if !dt.is_finite() || dt < 0.0 {
            return Err(PlayerError::InvalidDelta { got: dt });
        }
        if self.state != PlaybackState::Playing {
            return Ok(fired);
        }
        let delta = self.speed * dt;
        if !delta.is_finite() {
            return Err(PlayerError::DeltaOverflow);
        }
        let duration = self.timeline.duration();
        debug_assert!(
            self.local_time <= duration,
            "local time never exceeds duration"
        );

        let mut cursor = self.local_time;
        let mut remaining = delta;
        loop {
            let target = cursor + remaining;
            if target < duration {
                fired.extend(self.timeline.events_between(cursor, target));
                cursor = target;
                self.local_time = cursor;
                return Ok(fired);
            }
            // This cycle's tail window (cursor, duration].
            fired.extend(self.timeline.events_between(cursor, duration));
            let carry = target - duration;
            match self.loop_mode {
                LoopMode::Once => {
                    self.local_time = duration;
                    self.state = PlaybackState::Finished;
                    return Ok(fired);
                }
                LoopMode::Loop => {
                    // Fire every FULLY traversed cycle exactly once per
                    // cycle (a delta larger than the whole timeline is a
                    // pathological driver, handled without loss).
                    let full_cycles = (carry / duration).floor();
                    for _ in 0..(full_cycles as u32) {
                        fired.extend(self.timeline.events_between(0.0, duration));
                    }
                    cursor = 0.0;
                    remaining = carry - full_cycles * duration;
                }
            }
        }
    }

    /// Samples every track at the current local time, in deterministic
    /// (target, channel) order.
    #[must_use]
    pub fn sample_all(&self) -> Vec<(&TargetChannel, SampledValue)> {
        self.timeline.sample_all(self.local_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::Channel;
    use crate::track::{FloatKey, FloatTrack, Interpolation};

    fn simple_timeline() -> Arc<Timeline> {
        Arc::new(
            Timeline::builder(2.0)
                .float_track(
                    "cam",
                    Channel::Fade,
                    FloatTrack::new(
                        vec![
                            FloatKey {
                                time: 0.0,
                                value: 0.0,
                            },
                            FloatKey {
                                time: 2.0,
                                value: 1.0,
                            },
                        ],
                        Interpolation::Linear,
                    )
                    .unwrap(),
                )
                .expect("track ok")
                .build()
                .expect("timeline ok"),
        )
    }

    fn evented_timeline() -> Arc<Timeline> {
        Arc::new(
            Timeline::builder(2.0)
                .float_track(
                    "cam",
                    Channel::Fade,
                    FloatTrack::new(
                        vec![FloatKey {
                            time: 0.0,
                            value: 0.0,
                        }],
                        Interpolation::Linear,
                    )
                    .unwrap(),
                )
                .expect("track ok")
                .event(0.5, "half")
                .expect("ok")
                .event(1.0, "one")
                .expect("ok")
                .event(2.0, "end")
                .expect("ok")
                .build()
                .expect("ok"),
        )
    }

    fn playing(timeline: Arc<Timeline>) -> CinematicPlayer {
        let mut p = CinematicPlayer::new(timeline);
        p.play();
        p
    }

    #[test]
    fn advance_on_idle_is_noop() {
        let mut p = CinematicPlayer::new(simple_timeline());
        let fired = p.advance(0.5).expect("ok");
        assert!(fired.is_empty());
        assert_eq!(p.state(), PlaybackState::Idle);
        assert_eq!(p.local_time(), 0.0);
    }

    #[test]
    fn negative_and_non_finite_delta_are_loud() {
        let mut p = playing(simple_timeline());
        assert!(matches!(
            p.advance(-0.1),
            Err(PlayerError::InvalidDelta { .. })
        ));
        assert!(matches!(
            p.advance(f32::NAN),
            Err(PlayerError::InvalidDelta { .. })
        ));
        // Overflow to infinity is its own loud error.
        p.set_speed(f32::MAX / 2.0).unwrap();
        assert_eq!(p.advance(4.0), Err(PlayerError::DeltaOverflow));
    }

    #[test]
    fn events_fire_once_on_half_open_windows() {
        let mut p = playing(evented_timeline());
        // (0, 0.5]: fires "half" exactly at the boundary.
        let fired = p.advance(0.5).unwrap();
        assert_eq!(names(&fired), vec!["half"]);
        // (0.5, 1.0]: fires "one".
        let fired = p.advance(0.5).unwrap();
        assert_eq!(names(&fired), vec!["one"]);
        // (1.0, 1.5]: nothing.
        assert!(p.advance(0.5).unwrap().is_empty());
        // (1.5, 2.0]: fires "end", finishes.
        let fired = p.advance(0.5).unwrap();
        assert_eq!(names(&fired), vec!["end"]);
        assert_eq!(p.state(), PlaybackState::Finished);
        assert_eq!(p.local_time(), 2.0);
        // Finished players stay put.
        assert!(p.advance(0.5).unwrap().is_empty());
        assert_eq!(p.local_time(), 2.0);
    }

    #[test]
    fn once_finishes_clamped_at_duration() {
        let mut p = playing(simple_timeline());
        let fired = p.advance(2.5).unwrap();
        assert!(fired.is_empty());
        assert_eq!(p.state(), PlaybackState::Finished);
        assert_eq!(p.local_time(), 2.0);
        assert_eq!(p.sample_all()[0].1, SampledValue::Float(1.0));
    }

    #[test]
    fn loop_wraps_and_fires_tail_then_head() {
        let mut p = playing(evented_timeline());
        p.set_loop_mode(LoopMode::Loop);
        // Advance 1.25s: no boundary crossed yet — (0, 1.25] fires
        // half (0.5) and one (1.0), not end (2.0).
        let fired = p.advance(1.25).unwrap();
        assert_eq!(names(&fired), vec!["half", "one"]);
        assert_eq!(p.local_time(), 1.25);
        // Advance 0.75s more: lands exactly on the boundary —
        // (1.25, 2.0] fires "end", then wraps to exactly 0.
        let fired = p.advance(0.75).unwrap();
        assert_eq!(names(&fired), vec!["end"]);
        assert_eq!(p.local_time(), 0.0);
        // Next cycle re-fires on crossing — once per cycle.
        let fired = p.advance(1.0).unwrap();
        assert_eq!(names(&fired), vec!["half", "one"]);
        assert_eq!(p.local_time(), 1.0);
    }

    #[test]
    fn pathological_delta_fires_every_cycle() {
        let mut p = playing(evented_timeline());
        p.set_loop_mode(LoopMode::Loop);
        // delta = 5s over a 2s timeline: tail (0,2] fires 3 events,
        // full_cycles = floor(3/2) = 1 fires 3 more, remaining 1.0 →
        // head (0,1] fires 2. Total 8.
        let fired = p.advance(5.0).unwrap();
        assert_eq!(
            names(&fired),
            vec![
                "half", "one", "end", // tail (0,2]
                "half", "one", "end", // one full cycle
                "half", "one", // head (0,1]
            ]
        );
        assert_eq!(p.local_time(), 1.0);
    }

    #[test]
    fn speed_scales_traversal_deterministically() {
        let mut p = playing(simple_timeline());
        p.set_speed(2.0).unwrap();
        p.advance(0.5).unwrap(); // traverses 1.0s of timeline
        assert_eq!(p.local_time(), 1.0);
        p.set_speed(0.5).unwrap();
        p.advance(0.5).unwrap(); // traverses 0.25s
        assert_eq!(p.local_time(), 1.25);
    }

    #[test]
    fn speed_zero_and_non_finite_rejected() {
        let mut p = CinematicPlayer::new(simple_timeline());
        assert!(matches!(
            p.set_speed(0.0),
            Err(PlayerError::InvalidSpeed { .. })
        ));
        assert!(matches!(
            p.set_speed(f32::NAN),
            Err(PlayerError::InvalidSpeed { .. })
        ));
    }

    #[test]
    fn pause_holds_and_play_resumes() {
        let mut p = playing(simple_timeline());
        p.advance(0.5).unwrap();
        p.pause();
        assert_eq!(p.state(), PlaybackState::Paused);
        assert!(p.advance(0.5).unwrap().is_empty());
        assert_eq!(p.local_time(), 0.5);
        p.play();
        assert_eq!(p.state(), PlaybackState::Playing);
        p.advance(0.5).unwrap();
        assert_eq!(p.local_time(), 1.0);
    }

    #[test]
    fn play_after_finish_restarts_from_zero() {
        let mut p = playing(simple_timeline());
        p.advance(2.5).unwrap();
        assert_eq!(p.state(), PlaybackState::Finished);
        p.play();
        assert_eq!(p.state(), PlaybackState::Playing);
        assert_eq!(p.local_time(), 0.0);
    }

    #[test]
    fn stop_resets_to_idle() {
        let mut p = playing(simple_timeline());
        p.advance(1.0).unwrap();
        p.stop();
        assert_eq!(p.state(), PlaybackState::Idle);
        assert_eq!(p.local_time(), 0.0);
        p.play();
        p.advance(0.1).unwrap();
        assert_eq!(p.local_time(), 0.1);
    }

    #[test]
    fn seek_is_exact_and_validated() {
        let mut p = playing(evented_timeline());
        p.seek(1.5).unwrap();
        assert_eq!(p.local_time(), 1.5);
        // Seeking PAST a crossing does not fire it (seek is not playback).
        let fired = p.advance(0.1).unwrap();
        assert!(fired.is_empty());
        assert!(matches!(
            p.seek(2.1),
            Err(PlayerError::SeekOutOfRange { .. })
        ));
    }

    fn names(events: &[TimelineEvent]) -> Vec<&str> {
        events.iter().map(|e| e.name.as_str()).collect()
    }
}
