//! Typed keyframe tracks with deterministic interpolation.
//!
//! The sampling contract mirrors the `gens-animation` clip vocabulary
//! where the semantics agree (sorted keys, strictly increasing times,
//! clamp at both ends, `Step` holds the left key) and tightens it in two
//! places that matter for cinematics:
//!
//! - an **empty track is a construction error** (a cinematic that would
//!   silently sample a default pose is a bug, not a default), and
//! - **non-finite key values are rejected at construction** — a NaN that
//!   reaches a camera pose poisons every downstream hash silently; here
//!   it is loud at build time instead.
//!
//! Interpolation is a pure function of `(keys, time)`: no interior
//! state, no clocks. `Smoothstep` eases the normalized segment position
//! with `t*t*(3-2t)` — plain IEEE-754 multiply/add, bit-identical across
//! platforms (ADR-0030).

use gens_math::{Quat, Vec3};
use thiserror::Error;

/// How values between neighboring keys are interpolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interpolation {
    /// Linear interpolation between the two neighboring keys (default).
    #[default]
    Linear,
    /// Hold the left key's value until the next key is reached.
    Step,
    /// Ease the normalized segment position with `t*t*(3-2t)` before
    /// linear interpolation — smooth acceleration in, smooth out.
    Smoothstep,
}

/// Errors from track construction.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TrackError {
    /// Keys must be sorted by strictly increasing time.
    #[error("track keys must be strictly increasing in time (bad pair at index {index})")]
    UnsortedKeys {
        /// Index of the first key whose time does not exceed its predecessor's.
        index: usize,
    },
    /// A track with no keys would silently sample a default value.
    #[error("track must contain at least one key")]
    EmptyTrack,
    /// Non-finite values would poison every downstream hash.
    #[error("track key at index {index} has a non-finite value")]
    NonFiniteValue {
        /// Index of the offending key.
        index: usize,
    },
}

/// A scalar keyframe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatKey {
    /// Time in seconds from the timeline start.
    pub time: f32,
    /// Value at that time.
    pub value: f32,
}

/// A vector keyframe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3Key {
    /// Time in seconds from the timeline start.
    pub time: f32,
    /// Value at that time.
    pub value: Vec3,
}

/// A rotation keyframe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuatKey {
    /// Time in seconds from the timeline start.
    pub time: f32,
    /// Value at that time.
    pub value: Quat,
}

/// Normalized segment position for smoothstep easing: `t*t*(3-2t)`.
///
/// Pure arithmetic on an already-clamped `t` in `[0, 1]`; exposed for
/// tests and for callers that want the same easing on their own scalars.
#[must_use]
pub fn smoothstep(t: f32) -> f32 {
    debug_assert!((0.0..=1.0).contains(&t), "smoothstep expects t in [0,1]");
    t * t * (3.0 - 2.0 * t)
}

/// Sorts nothing itself: validates an already-sorted key list — at least
/// one key, strictly increasing times.
fn validate_sorted_times<T>(keys: &[T], time_of: impl Fn(&T) -> f32) -> Result<(), TrackError> {
    if keys.is_empty() {
        return Err(TrackError::EmptyTrack);
    }
    for (i, pair) in keys.windows(2).enumerate() {
        if time_of(&pair[1]) <= time_of(&pair[0]) {
            return Err(TrackError::UnsortedKeys { index: i + 1 });
        }
    }
    Ok(())
}

/// Rejects any non-finite component, reporting the key's index.
fn validate_finite_components<T>(
    keys: &[T],
    components_of: impl Fn(&T) -> Vec<f32>,
) -> Result<(), TrackError> {
    for (i, k) in keys.iter().enumerate() {
        for c in components_of(k) {
            if !c.is_finite() {
                return Err(TrackError::NonFiniteValue { index: i });
            }
        }
    }
    Ok(())
}

/// Keyframes for one scalar channel (FOV, fade level, exposure, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct FloatTrack {
    keys: Vec<FloatKey>,
    interpolation: Interpolation,
}

impl FloatTrack {
    /// Builds a track from unsorted key input, validating after sorting.
    ///
    /// # Errors
    /// [`TrackError::EmptyTrack`] with no keys,
    /// [`TrackError::UnsortedKeys`] when two keys share a time,
    /// [`TrackError::NonFiniteValue`] on any non-finite value.
    pub fn new(mut keys: Vec<FloatKey>, interpolation: Interpolation) -> Result<Self, TrackError> {
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        validate_sorted_times(&keys, |k| k.time)?;
        validate_finite_components(&keys, |k| vec![k.value])?;
        Ok(Self {
            keys,
            interpolation,
        })
    }

    /// The track's interpolation mode.
    #[must_use]
    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    /// All keys in sampling order (sorted by time).
    #[must_use]
    pub fn keys(&self) -> &[FloatKey] {
        &self.keys
    }

    /// Sampled value at `time` with clamp-at-ends semantics.
    #[must_use]
    pub fn sample(&self, time: f32) -> f32 {
        let keys = &self.keys;
        if time <= keys[0].time {
            return keys[0].value;
        }
        let last = keys.len() - 1;
        if time >= keys[last].time {
            return keys[last].value;
        }
        // Find the last key with time <= requested time.
        let i = keys
            .iter()
            .rposition(|k| k.time <= time)
            .expect("time is within key range");
        let (a, b) = (&keys[i], &keys[i + 1]);
        match self.interpolation {
            Interpolation::Step => a.value,
            Interpolation::Linear => {
                let t = (time - a.time) / (b.time - a.time);
                a.value + (b.value - a.value) * t
            }
            Interpolation::Smoothstep => {
                let t = smoothstep((time - a.time) / (b.time - a.time));
                a.value + (b.value - a.value) * t
            }
        }
    }
}

/// Keyframes for one vector channel (translation, scale, color-as-xyz).
#[derive(Debug, Clone, PartialEq)]
pub struct Vec3Track {
    keys: Vec<Vec3Key>,
    interpolation: Interpolation,
}

impl Vec3Track {
    /// Builds a track from unsorted key input, validating after sorting.
    ///
    /// # Errors
    /// As [`FloatTrack::new`], with finiteness checked per component.
    pub fn new(mut keys: Vec<Vec3Key>, interpolation: Interpolation) -> Result<Self, TrackError> {
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        validate_sorted_times(&keys, |k| k.time)?;
        validate_finite_components(&keys, |k| vec![k.value.x, k.value.y, k.value.z])?;
        Ok(Self {
            keys,
            interpolation,
        })
    }

    /// The track's interpolation mode.
    #[must_use]
    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    /// All keys in sampling order (sorted by time).
    #[must_use]
    pub fn keys(&self) -> &[Vec3Key] {
        &self.keys
    }

    /// Sampled value at `time` with clamp-at-ends semantics.
    #[must_use]
    pub fn sample(&self, time: f32) -> Vec3 {
        let keys = &self.keys;
        if time <= keys[0].time {
            return keys[0].value;
        }
        let last = keys.len() - 1;
        if time >= keys[last].time {
            return keys[last].value;
        }
        let i = keys
            .iter()
            .rposition(|k| k.time <= time)
            .expect("time is within key range");
        let (a, b) = (&keys[i], &keys[i + 1]);
        match self.interpolation {
            Interpolation::Step => a.value,
            Interpolation::Linear => {
                let t = (time - a.time) / (b.time - a.time);
                Vec3::lerp(a.value, b.value, t)
            }
            Interpolation::Smoothstep => {
                let t = smoothstep((time - a.time) / (b.time - a.time));
                Vec3::lerp(a.value, b.value, t)
            }
        }
    }
}

/// Keyframes for one rotation channel.
#[derive(Debug, Clone, PartialEq)]
pub struct QuatTrack {
    keys: Vec<QuatKey>,
    interpolation: Interpolation,
}

impl QuatTrack {
    /// Builds a track from unsorted key input, validating after sorting.
    ///
    /// # Errors
    /// As [`FloatTrack::new`], with finiteness checked per component.
    pub fn new(mut keys: Vec<QuatKey>, interpolation: Interpolation) -> Result<Self, TrackError> {
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        validate_sorted_times(&keys, |k| k.time)?;
        validate_finite_components(&keys, |k| vec![k.value.x, k.value.y, k.value.z, k.value.w])?;
        Ok(Self {
            keys,
            interpolation,
        })
    }

    /// The track's interpolation mode.
    #[must_use]
    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    /// All keys in sampling order (sorted by time).
    #[must_use]
    pub fn keys(&self) -> &[QuatKey] {
        &self.keys
    }

    /// Sampled rotation at `time` with clamp-at-ends semantics.
    ///
    /// Between keys the rotation is the shortest-arc slerp of the two
    /// neighboring quaternions (`Quat::slerp`); `Step` holds the left
    /// key; `Smoothstep` eases the slerp parameter.
    #[must_use]
    pub fn sample(&self, time: f32) -> Quat {
        let keys = &self.keys;
        if time <= keys[0].time {
            return keys[0].value;
        }
        let last = keys.len() - 1;
        if time >= keys[last].time {
            return keys[last].value;
        }
        let i = keys
            .iter()
            .rposition(|k| k.time <= time)
            .expect("time is within key range");
        let (a, b) = (&keys[i], &keys[i + 1]);
        match self.interpolation {
            Interpolation::Step => a.value,
            Interpolation::Linear => {
                Quat::slerp(a.value, b.value, (time - a.time) / (b.time - a.time))
            }
            Interpolation::Smoothstep => Quat::slerp(
                a.value,
                b.value,
                smoothstep((time - a.time) / (b.time - a.time)),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::{quat, vec3};

    fn keys(times_values: &[(f32, f32)]) -> Vec<FloatKey> {
        times_values
            .iter()
            .map(|&(t, v)| FloatKey { time: t, value: v })
            .collect()
    }

    #[test]
    fn construction_rejects_empty_duplicate_and_unsorted() {
        assert_eq!(
            FloatTrack::new(vec![], Interpolation::Linear),
            Err(TrackError::EmptyTrack)
        );
        // After sorting, a duplicate time is a non-increasing pair.
        let dup = keys(&[(0.0, 1.0), (1.0, 2.0), (1.0, 3.0)]);
        assert_eq!(
            FloatTrack::new(dup, Interpolation::Linear),
            Err(TrackError::UnsortedKeys { index: 2 })
        );
    }

    #[test]
    fn construction_rejects_non_finite_values() {
        let bad = keys(&[(0.0, 1.0), (1.0, f32::NAN)]);
        assert_eq!(
            FloatTrack::new(bad, Interpolation::Linear),
            Err(TrackError::NonFiniteValue { index: 1 })
        );
        let inf = keys(&[(0.0, f32::INFINITY)]);
        assert_eq!(
            FloatTrack::new(inf, Interpolation::Linear),
            Err(TrackError::NonFiniteValue { index: 0 })
        );
    }

    #[test]
    fn input_order_does_not_matter() {
        let a = FloatTrack::new(
            keys(&[(0.0, 0.0), (2.0, 2.0), (1.0, 1.0)]),
            Interpolation::Linear,
        )
        .expect("valid after sort");
        let b = FloatTrack::new(
            keys(&[(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)]),
            Interpolation::Linear,
        )
        .expect("already sorted");
        assert_eq!(a, b);
    }

    #[test]
    fn clamps_at_both_ends() {
        let t = FloatTrack::new(keys(&[(1.0, 10.0), (3.0, 30.0)]), Interpolation::Linear).unwrap();
        assert_eq!(t.sample(0.0), 10.0);
        assert_eq!(t.sample(1.0), 10.0);
        assert_eq!(t.sample(3.0), 30.0);
        assert_eq!(t.sample(99.0), 30.0);
    }

    #[test]
    fn linear_midpoint_is_exact() {
        let t = FloatTrack::new(keys(&[(0.0, 0.0), (2.0, 4.0)]), Interpolation::Linear).unwrap();
        // 0.5 of [0,2] and values 0..4: the arithmetic is exact in f32.
        assert_eq!(t.sample(1.0), 2.0);
    }

    #[test]
    fn step_holds_left_value() {
        let t = FloatTrack::new(keys(&[(0.0, 0.0), (2.0, 4.0)]), Interpolation::Step).unwrap();
        assert_eq!(t.sample(1.999), 0.0);
        assert_eq!(t.sample(2.0), 4.0);
    }

    #[test]
    fn smoothstep_eases_but_matches_keys() {
        let lin = FloatTrack::new(keys(&[(0.0, 0.0), (1.0, 1.0)]), Interpolation::Linear).unwrap();
        let sm =
            FloatTrack::new(keys(&[(0.0, 0.0), (1.0, 1.0)]), Interpolation::Smoothstep).unwrap();
        // Eased curve is below linear in the first half, above in the second.
        assert!(sm.sample(0.25) < lin.sample(0.25));
        assert!(sm.sample(0.75) > lin.sample(0.75));
        // Key times still hit exact values.
        assert_eq!(sm.sample(0.0), 0.0);
        assert_eq!(sm.sample(1.0), 1.0);
        // smoothstep(0.5) = 0.5 exactly.
        assert_eq!(sm.sample(0.5), 0.5);
    }

    #[test]
    fn vec3_track_validates_each_component() {
        let bad = vec![
            Vec3Key {
                time: 0.0,
                value: vec3(1.0, 1.0, 1.0),
            },
            Vec3Key {
                time: 1.0,
                value: vec3(1.0, f32::NAN, 1.0),
            },
        ];
        assert_eq!(
            Vec3Track::new(bad, Interpolation::Linear),
            Err(TrackError::NonFiniteValue { index: 1 })
        );
    }

    #[test]
    fn vec3_midpoint_is_exact() {
        let t = Vec3Track::new(
            vec![
                Vec3Key {
                    time: 0.0,
                    value: Vec3::ZERO,
                },
                Vec3Key {
                    time: 2.0,
                    value: vec3(4.0, -2.0, 8.0),
                },
            ],
            Interpolation::Linear,
        )
        .unwrap();
        assert_eq!(t.sample(1.0), vec3(2.0, -1.0, 4.0));
    }

    #[test]
    fn quat_slerp_midpoint_rotates_halfway() {
        let identity = quat(0.0, 0.0, 0.0, 1.0);
        let half_turn = quat(0.0, 1.0, 0.0, 0.0); // 180 degrees around Y
        let t = QuatTrack::new(
            vec![
                QuatKey {
                    time: 0.0,
                    value: identity,
                },
                QuatKey {
                    time: 2.0,
                    value: half_turn,
                },
            ],
            Interpolation::Linear,
        )
        .unwrap();
        let mid = t.sample(1.0);
        // Halfway between identity and a 180° Y turn is a 90° Y turn.
        let expected = quat(0.0, (2.0_f32).sqrt() / 2.0, 0.0, (2.0_f32).sqrt() / 2.0);
        assert!(
            (mid.dot(expected) - 1.0).abs() < 1e-5,
            "mid {mid:?} vs {expected:?}"
        );
    }

    #[test]
    fn sampling_is_stateless_and_repeatable() {
        let t = FloatTrack::new(
            keys(&[(0.0, 1.0), (1.0, 2.0), (4.0, -1.0)]),
            Interpolation::Smoothstep,
        )
        .unwrap();
        let first: Vec<f32> = (0..100).map(|i| t.sample(i as f32 * 0.05)).collect();
        let second: Vec<f32> = (0..100).map(|i| t.sample(i as f32 * 0.05)).collect();
        assert_eq!(first, second, "sampling must be a pure function of time");
    }
}
