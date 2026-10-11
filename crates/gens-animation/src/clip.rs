//! Keyframe tracks and animation clips.
//!
//! A [`Clip`] maps joint indices to per-channel [`JointTrack`]s
//! (translation / rotation / scale). Sampling is a pure function of time:
//!
//! - before the first key → the first key's value,
//! - between keys → linear (`Vec3::lerp`) or shortest-arc slerp
//!   (`Quat::slerp`) by the track's [`Interpolation`], or the left key's
//!   value for `Step`,
//! - at/after the last key → the last key's value (`sample`),
//!
//! Looping is the caller's concern through [`Clip::sample_looped`], which
//! reduces the time modulo the duration — deterministic pure arithmetic,
//! no hidden state anywhere (R-013 discipline).

use std::collections::BTreeMap;

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
}

/// Errors from track/clip validation.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipError {
    /// Track keys must be sorted by strictly increasing time.
    #[error("track keys must be strictly increasing in time (bad pair at index {index})")]
    UnsortedKeys {
        /// Index of the first key whose time does not exceed its predecessor's.
        index: usize,
    },
    /// A clip must have a positive duration to be loopable.
    #[error("clip duration must be finite and > 0, got {duration}")]
    InvalidDuration {
        /// The offending duration.
        duration: f32,
    },
    /// A clip's tracks must reference existing joints.
    #[error(
        "track references joint {joint} but the clip is bound to a {joint_count}-joint skeleton"
    )]
    UnknownJoint {
        /// The offending joint index.
        joint: usize,
        /// Number of joints the clip is bound to.
        joint_count: usize,
    },
}

/// A scalar-vector keyframe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VecKey {
    /// Time in seconds from the clip start.
    pub time: f32,
    /// Value at that time.
    pub value: Vec3,
}

/// A rotation keyframe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuatKey {
    /// Time in seconds from the clip start.
    pub time: f32,
    /// Value at that time.
    pub value: Quat,
}

/// Keyframes for one vector channel (translation or scale).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VecTrack {
    keys: Vec<VecKey>,
    interpolation: Interpolation,
}

impl VecTrack {
    /// Builds a track from unsorted key input, validating after sorting.
    ///
    /// # Errors
    /// [`ClipError::UnsortedKeys`] when two keys share the same time
    /// (after sorting, strict increase is checkable deterministically).
    pub fn new(mut keys: Vec<VecKey>, interpolation: Interpolation) -> Result<Self, ClipError> {
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        Self::validate_sorted(&keys)?;
        Ok(Self {
            keys,
            interpolation,
        })
    }

    fn validate_sorted(keys: &[VecKey]) -> Result<(), ClipError> {
        for (i, pair) in keys.windows(2).enumerate() {
            if pair[1].time <= pair[0].time {
                return Err(ClipError::UnsortedKeys { index: i + 1 });
            }
        }
        Ok(())
    }

    /// Sampled value at `time` with clamp-at-ends semantics.
    #[must_use]
    pub fn sample(&self, time: f32) -> Vec3 {
        let keys = &self.keys;
        if keys.is_empty() {
            return Vec3::ONE;
        }
        if time <= keys[0].time {
            return keys[0].value;
        }
        if time >= keys[keys.len() - 1].time {
            return keys[keys.len() - 1].value;
        }
        // Find the last key with time <= requested time.
        let upper = keys.partition_point(|k| k.time <= time);
        let left = &keys[upper - 1];
        let right = &keys[upper];
        match self.interpolation {
            Interpolation::Step => left.value,
            Interpolation::Linear => {
                let span = right.time - left.time;
                let t = (time - left.time) / span;
                left.value.lerp(right.value, t)
            }
        }
    }

    /// Number of keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// `true` when the track has no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The track's keys in sorted order.
    #[must_use]
    pub fn keys(&self) -> &[VecKey] {
        &self.keys
    }

    /// The track's interpolation mode.
    #[must_use]
    pub const fn interpolation(&self) -> Interpolation {
        self.interpolation
    }
}

/// Keyframes for one rotation channel.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuatTrack {
    keys: Vec<QuatKey>,
    interpolation: Interpolation,
}

impl QuatTrack {
    /// Builds a rotation track (sorted by time, validated).
    ///
    /// # Errors
    /// [`ClipError::UnsortedKeys`] when two keys share a time.
    pub fn new(mut keys: Vec<QuatKey>, interpolation: Interpolation) -> Result<Self, ClipError> {
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        for (i, pair) in keys.windows(2).enumerate() {
            if pair[1].time <= pair[0].time {
                return Err(ClipError::UnsortedKeys { index: i + 1 });
            }
        }
        Ok(Self {
            keys,
            interpolation,
        })
    }

    /// Sampled rotation at `time`; interpolation is shortest-arc slerp.
    #[must_use]
    pub fn sample(&self, time: f32) -> Quat {
        let keys = &self.keys;
        if keys.is_empty() {
            return Quat::IDENTITY;
        }
        if time <= keys[0].time {
            return keys[0].value;
        }
        if time >= keys[keys.len() - 1].time {
            return keys[keys.len() - 1].value;
        }
        let upper = keys.partition_point(|k| k.time <= time);
        let left = &keys[upper - 1];
        let right = &keys[upper];
        match self.interpolation {
            Interpolation::Step => left.value,
            Interpolation::Linear => {
                let span = right.time - left.time;
                let t = (time - left.time) / span;
                left.value.slerp(right.value, t)
            }
        }
    }

    /// Number of keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// `true` when the track has no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The track's keys in sorted order.
    #[must_use]
    pub fn keys(&self) -> &[QuatKey] {
        &self.keys
    }

    /// The track's interpolation mode.
    #[must_use]
    pub const fn interpolation(&self) -> Interpolation {
        self.interpolation
    }
}

/// The three optional channels of one joint inside a clip.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JointTrack {
    /// Position keys; `None` leaves the joint's translation untouched.
    pub translation: Option<VecTrack>,
    /// Rotation keys; `None` leaves the joint's rotation untouched.
    pub rotation: Option<QuatTrack>,
    /// Scale keys; `None` leaves the joint's scale untouched.
    pub scale: Option<VecTrack>,
}

impl JointTrack {
    /// `true` when every channel is absent (the track would do nothing).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.translation.is_none() && self.rotation.is_none() && self.scale.is_none()
    }
}

/// A named, duration-bounded set of joint tracks.
///
/// Sampling writes only the joints the clip has tracks for; all other
/// joints keep whatever pose the caller started from — compose clips on
/// a [`crate::skeleton::LocalPose`] initialized to identity or rest.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    name: String,
    duration: f32,
    tracks: BTreeMap<usize, JointTrack>,
}

impl Clip {
    /// Builds a validated clip.
    ///
    /// # Errors
    /// - [`ClipError::InvalidDuration`] when `duration <= 0` or NaN.
    /// - [`ClipError::UnsortedKeys`] from any track's construction.
    pub fn new(
        name: impl Into<String>,
        duration: f32,
        tracks: BTreeMap<usize, JointTrack>,
    ) -> Result<Self, ClipError> {
        if !duration.is_finite() || duration <= 0.0 {
            return Err(ClipError::InvalidDuration { duration });
        }
        for (joint, track) in &tracks {
            if let Some(t) = &track.translation {
                let _ = VecTrack::new(t.keys().to_vec(), t.interpolation())?;
            }
            if let Some(t) = &track.rotation {
                let _ = QuatTrack::new(t.keys().to_vec(), t.interpolation())?;
            }
            if let Some(t) = &track.scale {
                let _ = VecTrack::new(t.keys().to_vec(), t.interpolation())?;
            }
            let _ = joint;
        }
        Ok(Self {
            name: name.into(),
            duration,
            tracks,
        })
    }

    /// Convenience constructor for translation-only tracks.
    ///
    /// # Errors
    /// Same as [`Self::new`].
    pub fn translation(
        name: impl Into<String>,
        duration: f32,
        joint: usize,
        keys: Vec<VecKey>,
    ) -> Result<Self, ClipError> {
        let mut tracks = BTreeMap::new();
        tracks.insert(
            joint,
            JointTrack {
                translation: Some(VecTrack::new(keys, Interpolation::Linear)?),
                ..JointTrack::default()
            },
        );
        Self::new(name, duration, tracks)
    }

    /// Convenience constructor for rotation-only tracks.
    ///
    /// # Errors
    /// Same as [`Self::new`].
    pub fn rotation(
        name: impl Into<String>,
        duration: f32,
        joint: usize,
        keys: Vec<QuatKey>,
    ) -> Result<Self, ClipError> {
        let mut tracks = BTreeMap::new();
        tracks.insert(
            joint,
            JointTrack {
                rotation: Some(QuatTrack::new(keys, Interpolation::Linear)?),
                ..JointTrack::default()
            },
        );
        Self::new(name, duration, tracks)
    }

    /// Clip name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Clip length in seconds.
    #[must_use]
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Joints this clip animates, in index order.
    pub fn joints(&self) -> impl Iterator<Item = usize> + '_ {
        self.tracks.keys().copied()
    }

    /// Read access to one joint's track.
    #[must_use]
    pub fn track(&self, joint: usize) -> Option<&JointTrack> {
        self.tracks.get(&joint)
    }

    /// Samples the clip at `time` (clamped to `[0, duration]`) into `pose`.
    ///
    /// Joints without tracks are left exactly as they are in `pose`.
    /// `time` values outside the clip are clamped, not wrapped — use
    /// [`Self::sample_looped`] for wrap-around playback.
    pub fn sample_clamped(&self, time: f32, pose: &mut crate::skeleton::LocalPose) {
        let t = time.clamp(0.0, self.duration);
        for (&joint, track) in &self.tracks {
            if joint >= pose.joint_count() {
                continue; // clip bound to a smaller skeleton: skip honestly
            }
            let slot = pose.get_mut(joint);
            if let Some(tr) = &track.translation {
                slot.translation = tr.sample(t);
            }
            if let Some(tr) = &track.rotation {
                slot.rotation = tr.sample(t);
            }
            if let Some(tr) = &track.scale {
                slot.scale = tr.sample(t);
            }
        }
    }

    /// Samples the clip with time wrapped into `[0, duration)`.
    ///
    /// `time % duration` is pure arithmetic — same input, same output,
    /// deterministic within a process (R-013).
    pub fn sample_looped(&self, time: f32, pose: &mut crate::skeleton::LocalPose) {
        let wrapped = if self.duration > 0.0 {
            time % self.duration
        } else {
            0.0
        };
        self.sample_clamped(wrapped, pose);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::LocalPose;
    use gens_math::{Quat, vec3};

    const EPS: f32 = 1e-5;

    fn assert_vec3_close(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-4, "vectors not close: {a:?} vs {b:?}");
    }

    #[test]
    fn vec_track_clamps_before_first_and_after_last() {
        let track = VecTrack::new(
            vec![
                VecKey {
                    time: 1.0,
                    value: vec3(0.0, 0.0, 0.0),
                },
                VecKey {
                    time: 3.0,
                    value: vec3(10.0, 0.0, 0.0),
                },
            ],
            Interpolation::Linear,
        )
        .expect("valid");
        assert_vec3_close(track.sample(0.0), vec3(0.0, 0.0, 0.0));
        assert_vec3_close(track.sample(0.5), vec3(0.0, 0.0, 0.0));
        assert_vec3_close(track.sample(9.0), vec3(10.0, 0.0, 0.0));
    }

    #[test]
    fn vec_track_linear_midpoint() {
        let track = VecTrack::new(
            vec![
                VecKey {
                    time: 0.0,
                    value: vec3(0.0, 0.0, 0.0),
                },
                VecKey {
                    time: 2.0,
                    value: vec3(4.0, -8.0, 0.0),
                },
            ],
            Interpolation::Linear,
        )
        .expect("valid");
        assert_vec3_close(track.sample(1.0), vec3(2.0, -4.0, 0.0));
        assert_vec3_close(track.sample(0.5), vec3(1.0, -2.0, 0.0));
    }

    #[test]
    fn vec_track_piecewise_across_three_keys() {
        let track = VecTrack::new(
            vec![
                VecKey {
                    time: 0.0,
                    value: vec3(0.0, 0.0, 0.0),
                },
                VecKey {
                    time: 1.0,
                    value: vec3(2.0, 0.0, 0.0),
                },
                VecKey {
                    time: 3.0,
                    value: vec3(2.0, 4.0, 0.0),
                },
            ],
            Interpolation::Linear,
        )
        .expect("valid");
        assert_vec3_close(track.sample(0.5), vec3(1.0, 0.0, 0.0));
        assert_vec3_close(track.sample(2.0), vec3(2.0, 2.0, 0.0));
    }

    #[test]
    fn vec_track_step_holds_left_value() {
        let track = VecTrack::new(
            vec![
                VecKey {
                    time: 0.0,
                    value: vec3(1.0, 0.0, 0.0),
                },
                VecKey {
                    time: 2.0,
                    value: vec3(3.0, 0.0, 0.0),
                },
            ],
            Interpolation::Step,
        )
        .expect("valid");
        assert_vec3_close(track.sample(1.999), vec3(1.0, 0.0, 0.0));
        assert_vec3_close(track.sample(2.0), vec3(3.0, 0.0, 0.0));
    }

    #[test]
    fn vec_track_rejects_duplicate_times() {
        let sorted = vec![
            VecKey {
                time: 1.0,
                value: Vec3::ZERO,
            },
            VecKey {
                time: 1.0,
                value: Vec3::ONE,
            },
        ];
        assert_eq!(
            VecTrack::new(sorted, Interpolation::Linear),
            Err(ClipError::UnsortedKeys { index: 1 })
        );
    }

    #[test]
    fn vec_track_sorts_unordered_input() {
        let track = VecTrack::new(
            vec![
                VecKey {
                    time: 2.0,
                    value: vec3(2.0, 0.0, 0.0),
                },
                VecKey {
                    time: 0.0,
                    value: vec3(0.0, 0.0, 0.0),
                },
            ],
            Interpolation::Linear,
        )
        .expect("valid");
        assert_eq!(track.keys()[0].time, 0.0);
        assert_eq!(track.keys()[1].time, 2.0);
    }

    #[test]
    fn quat_track_slerps_shortest_arc() {
        // Endpoint is a true -90 deg z-rotation. (A -270 deg construction
        // canonicalizes to the SAME quaternion as +90 deg - unit quaternions
        // double-cover rotations - so "the long way" is not representable
        // from glam's canonical constructors.)
        let from = Quat::IDENTITY;
        let to = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        let track = QuatTrack::new(
            vec![
                QuatKey {
                    time: 0.0,
                    value: from,
                },
                QuatKey {
                    time: 1.0,
                    value: to,
                },
            ],
            Interpolation::Linear,
        )
        .expect("valid");
        let mid = track.sample(0.5);
        let expected = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_4);
        assert!(
            (mid.dot(expected) - 1.0).abs() < EPS,
            "expected -45 deg z rotation, got {mid:?}"
        );
    }

    #[test]
    fn quat_track_quarter_point() {
        let from = Quat::IDENTITY;
        let to = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let track = QuatTrack::new(
            vec![
                QuatKey {
                    time: 0.0,
                    value: from,
                },
                QuatKey {
                    time: 4.0,
                    value: to,
                },
            ],
            Interpolation::Linear,
        )
        .expect("valid");
        let quarter = track.sample(1.0);
        let expected = Quat::from_rotation_y(std::f32::consts::FRAC_PI_8);
        assert!((quarter.dot(expected) - 1.0).abs() < EPS);
    }

    #[test]
    fn empty_tracks_sample_to_defaults() {
        let track = VecTrack::default();
        assert!(track.is_empty());
        assert_vec3_close(track.sample(1.0), Vec3::ONE);
        let quat_track = QuatTrack::default();
        assert!(quat_track.sample(1.0).is_near_identity());
    }

    #[test]
    fn clip_rejects_non_positive_duration() {
        let err = Clip::new("bad", 0.0, BTreeMap::new()).unwrap_err();
        assert_eq!(err, ClipError::InvalidDuration { duration: 0.0 });
        assert!(matches!(
            Clip::new("bad", -1.0, BTreeMap::new()),
            Err(ClipError::InvalidDuration { duration: -1.0 })
        ));
    }

    #[test]
    fn clip_translation_sample_writes_only_target_joint() {
        let clip = Clip::translation(
            "slide",
            2.0,
            1,
            vec![
                VecKey {
                    time: 0.0,
                    value: vec3(0.0, 0.0, 0.0),
                },
                VecKey {
                    time: 2.0,
                    value: vec3(6.0, 0.0, 0.0),
                },
            ],
        )
        .expect("valid");

        let mut pose = LocalPose::identity(3);
        pose.get_mut(0).translation = vec3(99.0, 0.0, 0.0); // must survive
        clip.sample_clamped(1.0, &mut pose);
        assert_eq!(pose.get(0).translation, vec3(99.0, 0.0, 0.0));
        assert_vec3_close(pose.get(1).translation, vec3(3.0, 0.0, 0.0));
        assert_eq!(pose.get(2).translation, Vec3::ZERO);
    }

    #[test]
    fn clip_looped_wraps_time_and_is_deterministic() {
        let clip = Clip::translation(
            "loop",
            2.0,
            0,
            vec![
                VecKey {
                    time: 0.0,
                    value: vec3(0.0, 0.0, 0.0),
                },
                VecKey {
                    time: 2.0,
                    value: vec3(2.0, 0.0, 0.0),
                },
            ],
        )
        .expect("valid");

        let mut a = LocalPose::identity(1);
        let mut b = LocalPose::identity(1);
        clip.sample_looped(2.5, &mut a);
        clip.sample_looped(4.5, &mut b); // same phase, different absolute time
        assert_eq!(a, b, "same phase must sample identically");
        assert_vec3_close(a.get(0).translation, vec3(0.5, 0.0, 0.0));
    }

    #[test]
    fn clip_scale_channel_applies_through_compose() {
        let mut tracks = BTreeMap::new();
        tracks.insert(
            0,
            JointTrack {
                scale: Some(
                    VecTrack::new(
                        vec![
                            VecKey {
                                time: 0.0,
                                value: vec3(1.0, 1.0, 1.0),
                            },
                            VecKey {
                                time: 1.0,
                                value: vec3(3.0, 1.0, 1.0),
                            },
                        ],
                        Interpolation::Linear,
                    )
                    .expect("valid"),
                ),
                ..JointTrack::default()
            },
        );
        let clip = Clip::new("stretch", 1.0, tracks).expect("valid");
        let mut pose = LocalPose::identity(1);
        clip.sample_clamped(0.5, &mut pose);
        assert_vec3_close(pose.get(0).scale, vec3(2.0, 1.0, 1.0));
    }

    #[test]
    fn clip_at_exact_duration_equals_last_key() {
        let clip = Clip::translation(
            "once",
            2.0,
            0,
            vec![
                VecKey {
                    time: 0.0,
                    value: vec3(0.0, 0.0, 0.0),
                },
                VecKey {
                    time: 2.0,
                    value: vec3(5.0, 0.0, 0.0),
                },
            ],
        )
        .expect("valid");
        let mut pose = LocalPose::identity(1);
        clip.sample_clamped(2.0, &mut pose);
        assert_vec3_close(pose.get(0).translation, vec3(5.0, 0.0, 0.0));
    }

    #[test]
    fn clip_reports_joints_in_index_order() {
        let mut tracks = BTreeMap::new();
        tracks.insert(
            2,
            JointTrack {
                translation: Some(
                    VecTrack::new(
                        vec![VecKey {
                            time: 0.0,
                            value: Vec3::ZERO,
                        }],
                        Interpolation::Linear,
                    )
                    .expect("valid"),
                ),
                ..JointTrack::default()
            },
        );
        tracks.insert(0, JointTrack::default());
        let clip = Clip::new("multi", 1.0, tracks).expect("valid");
        let joints: Vec<usize> = clip.joints().collect();
        assert_eq!(
            joints,
            vec![0, 2],
            "BTreeMap iteration must be index-sorted"
        );
    }
}
