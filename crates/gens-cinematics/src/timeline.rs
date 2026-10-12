//! A cinematic timeline: named tracks over logical targets plus events.
//!
//! A [`Timeline`] is **pure content** — it knows nothing about entities,
//! worlds or clocks. Tracks are addressed by a logical target name
//! (e.g. `"camera"`, `"hero"`) and a [`Channel`]; at playback start the
//! embedding layer binds those names to real entities through an ordered
//! map, so one timeline can drive any scene. Events are named markers at
//! fixed times, fired by the player exactly when playback crosses them.
//!
//! Validation is complete at construction: duration is finite and > 0,
//! every track key lies in `[0, duration]`, events lie in `(0, duration]`
//! (a zero-time event could never fire on a first play — that would be a
//! silently dead marker), and duplicate target/channel pairs are
//! rejected loudly. Track iteration order is the [`BTreeMap`] order over
//! `(target, channel)` — deterministic on every platform.

use std::collections::BTreeMap;

use gens_math::{Quat, Vec3};
use thiserror::Error;

use crate::track::{FloatTrack, QuatTrack, TrackError, Vec3Track};

/// Which aspect of a target a track drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    /// Local-space translation of the bound entity.
    Translation,
    /// Local-space rotation of the bound entity.
    Rotation,
    /// Local-space scale of the bound entity.
    Scale,
    /// Camera field-of-view in degrees (bound target must be a camera).
    Fov,
    /// Free scalar the embedding layer maps to its own effect
    /// (fade-to-black opacity, exposure, ...).
    Fade,
}

/// A track bound to one `(target, channel)` pair.
#[derive(Debug, Clone, PartialEq)]
pub enum Track {
    /// Scalar channel.
    Float(FloatTrack),
    /// Vector channel.
    Vec3(Vec3Track),
    /// Rotation channel.
    Quat(QuatTrack),
}

impl Track {
    /// The channel kind this track can be bound to.
    #[must_use]
    pub fn kind(&self) -> ChannelKind {
        match self {
            Track::Float(_) => ChannelKind::Float,
            Track::Vec3(_) => ChannelKind::Vec3,
            Track::Quat(_) => ChannelKind::Quat,
        }
    }

    /// Sampled value at `time` (clamp-at-ends per track semantics).
    #[must_use]
    pub fn sample(&self, time: f32) -> SampledValue {
        match self {
            Track::Float(t) => SampledValue::Float(t.sample(time)),
            Track::Vec3(t) => SampledValue::Vec3(t.sample(time)),
            Track::Quat(t) => SampledValue::Quat(t.sample(time)),
        }
    }
}

/// The value kind a [`Channel`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    /// Scalar.
    Float,
    /// Vector.
    Vec3,
    /// Rotation.
    Quat,
}

/// One sampled value, tagged with its kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SampledValue {
    /// Scalar value.
    Float(f32),
    /// Vector value.
    Vec3(Vec3),
    /// Rotation value.
    Quat(Quat),
}

/// A full track address: which target, which channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TargetChannel {
    /// Logical target name (bound to an entity by the embedding layer).
    pub target: String,
    /// Which aspect of the target the track drives.
    pub channel: Channel,
}

/// A named event marker at a fixed time.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineEvent {
    /// Time in seconds from the timeline start, in `(0, duration]`.
    pub time: f32,
    /// Event name routed by the embedding layer.
    pub name: String,
}

/// Errors from timeline construction.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TimelineError {
    /// Duration must be finite and > 0.
    #[error("timeline duration must be finite and > 0, got {duration}")]
    InvalidDuration {
        /// The offending duration.
        duration: f32,
    },
    /// A timeline with no tracks would play nothing.
    #[error("timeline must contain at least one track")]
    EmptyTimeline,
    /// Two tracks claim the same target/channel.
    #[error("duplicate track for target '{target}' channel {channel:?}")]
    DuplicateTrack {
        /// The duplicated address.
        target: String,
        /// The duplicated channel.
        channel: Channel,
    },
    /// A track's kind must match its channel.
    #[error("channel {channel:?} needs a {expected:?} track, got {got:?}")]
    KindMismatch {
        /// The channel being bound.
        channel: Channel,
        /// The kind the channel requires.
        expected: ChannelKind,
        /// The kind the track actually is.
        got: ChannelKind,
    },
    /// A track key lies outside `[0, duration]`.
    #[error("track '{target}' channel {channel:?} has a key outside [0, duration]")]
    KeyOutOfRange {
        /// The offending track address.
        target: String,
        /// The offending channel.
        channel: Channel,
    },
    /// A track failed its own validation (empty / unsorted / non-finite).
    #[error("track '{target}' channel {channel:?} is invalid: {source}")]
    InvalidTrack {
        /// The offending track address.
        target: String,
        /// The offending channel.
        channel: Channel,
        /// The underlying track error.
        #[source]
        source: TrackError,
    },
    /// An event time must lie in `(0, duration]`.
    #[error("event '{name}' at {time} must lie in (0, duration]")]
    EventOutOfRange {
        /// The offending event name.
        name: String,
        /// The offending time.
        time: f32,
    },
}

/// Which logical target the cinematic directs as the camera, if any.
///
/// While a cinematic is playing, the embedding layer stages the camera
/// from this target's sampled pose (ADR-0030); on finish control hands
/// back to whatever drove the camera before.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraCut {
    /// The logical target name of the directed camera.
    pub target: String,
    /// Whether playback STARTS with a hard cut to the camera pose
    /// (true) or blends in over `blend_in` seconds (false — the
    /// embedding layer defines blending; the timeline only declares
    /// intent and the blend duration).
    pub hard_cut: bool,
    /// Blend-in duration in seconds when `hard_cut` is false. Zero is
    /// allowed and means "blend instantly" (identical to a hard cut).
    pub blend_in: f32,
}

/// A complete cinematic: tracks, events, duration and camera intent.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    duration: f32,
    tracks: BTreeMap<TargetChannel, Track>,
    events: Vec<TimelineEvent>,
    camera: Option<CameraCut>,
}

impl Timeline {
    /// The timeline duration in seconds.
    #[must_use]
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// All track addresses, in deterministic `(target, channel)` order.
    pub fn tracks(&self) -> impl Iterator<Item = (&TargetChannel, &Track)> {
        self.tracks.iter()
    }

    /// The track bound to `target`/`channel`, if any.
    #[must_use]
    pub fn track(&self, target: &str, channel: Channel) -> Option<&Track> {
        self.tracks.get(&TargetChannel {
            target: target.to_string(),
            channel,
        })
    }

    /// All event markers, in firing order (time, then insertion).
    #[must_use]
    pub fn events(&self) -> &[TimelineEvent] {
        &self.events
    }

    /// The camera direction intent, if the timeline declares one.
    #[must_use]
    pub fn camera(&self) -> Option<&CameraCut> {
        self.camera.as_ref()
    }

    /// Samples every track at `time`, in deterministic order.
    ///
    /// This is the single integration point the player and the
    /// embedding layer both use — a pure function of `(timeline, time)`.
    #[must_use]
    pub fn sample_all(&self, time: f32) -> Vec<(&TargetChannel, SampledValue)> {
        self.tracks
            .iter()
            .map(|(k, t)| (k, t.sample(time)))
            .collect()
    }

    /// Events with time in `(from, to]`, in firing order.
    ///
    /// The player's crossing primitive; exposed for tests and for
    /// seek-time inspection. `from <= to` is expected (not enforced
    /// beyond debug) — an empty window yields no events.
    #[must_use]
    pub fn events_between(&self, from: f32, to: f32) -> Vec<TimelineEvent> {
        debug_assert!(from <= to, "events_between expects from <= to");
        self.events
            .iter()
            .filter(|e| e.time > from && e.time <= to)
            .cloned()
            .collect()
    }

    /// Builder entry point for a timeline of `duration` seconds.
    #[must_use]
    pub fn builder(duration: f32) -> TimelineBuilder {
        TimelineBuilder::new(duration)
    }
}

/// Incremental builder for [`Timeline`] with fail-fast validation.
#[derive(Debug)]
pub struct TimelineBuilder {
    duration: f32,
    tracks: BTreeMap<TargetChannel, Track>,
    events: Vec<TimelineEvent>,
    camera: Option<CameraCut>,
}

impl TimelineBuilder {
    /// Starts a builder; duration is validated at `build`.
    #[must_use]
    pub fn new(duration: f32) -> TimelineBuilder {
        TimelineBuilder {
            duration,
            tracks: BTreeMap::new(),
            events: Vec::new(),
            camera: None,
        }
    }

    /// Declares which target is the directed camera (see [`CameraCut`]).
    #[must_use]
    pub fn camera(mut self, cut: CameraCut) -> TimelineBuilder {
        self.camera = Some(cut);
        self
    }

    /// Adds a track, validating kind match and key range immediately.
    ///
    /// # Errors
    /// [`TimelineError::DuplicateTrack`], [`TimelineError::KindMismatch`],
    /// [`TimelineError::KeyOutOfRange`] or [`TimelineError::InvalidTrack`].
    pub fn track(
        mut self,
        target: impl Into<String>,
        channel: Channel,
        track: Track,
    ) -> Result<TimelineBuilder, TimelineError> {
        let target = target.into();
        let address = TargetChannel {
            target: target.clone(),
            channel,
        };
        if self.tracks.contains_key(&address) {
            return Err(TimelineError::DuplicateTrack { target, channel });
        }
        let expected = match channel {
            Channel::Translation | Channel::Scale => ChannelKind::Vec3,
            Channel::Rotation => ChannelKind::Quat,
            Channel::Fov | Channel::Fade => ChannelKind::Float,
        };
        if track.kind() != expected {
            return Err(TimelineError::KindMismatch {
                channel,
                expected,
                got: track.kind(),
            });
        }
        validate_track_range(&track, self.duration).map_err(|source| match source {
            RangeError::OutOfRange => TimelineError::KeyOutOfRange {
                target: target.clone(),
                channel,
            },
            RangeError::Track(e) => TimelineError::InvalidTrack {
                target: target.clone(),
                channel,
                source: e,
            },
        })?;
        self.tracks.insert(address, track);
        Ok(self)
    }

    /// Adds a Float track (convenience for Fov/Fade channels).
    ///
    /// # Errors
    /// As [`TimelineBuilder::track`].
    pub fn float_track(
        self,
        target: impl Into<String>,
        channel: Channel,
        track: FloatTrack,
    ) -> Result<TimelineBuilder, TimelineError> {
        self.track(target, channel, Track::Float(track))
    }

    /// Adds a Vec3 track (convenience for Translation/Scale channels).
    ///
    /// # Errors
    /// As [`TimelineBuilder::track`].
    pub fn vec3_track(
        self,
        target: impl Into<String>,
        channel: Channel,
        track: Vec3Track,
    ) -> Result<TimelineBuilder, TimelineError> {
        self.track(target, channel, Track::Vec3(track))
    }

    /// Adds a Quat track (convenience for the Rotation channel).
    ///
    /// # Errors
    /// As [`TimelineBuilder::track`].
    pub fn quat_track(
        self,
        target: impl Into<String>,
        channel: Channel,
        track: QuatTrack,
    ) -> Result<TimelineBuilder, TimelineError> {
        self.track(target, channel, Track::Quat(track))
    }

    /// Adds an event marker, validating its time window immediately.
    ///
    /// # Errors
    /// [`TimelineError::EventOutOfRange`] when the time is not in
    /// `(0, duration]` or is non-finite.
    pub fn event(
        mut self,
        time: f32,
        name: impl Into<String>,
    ) -> Result<TimelineBuilder, TimelineError> {
        let name = name.into();
        if !time.is_finite() || time <= 0.0 || time > self.duration {
            return Err(TimelineError::EventOutOfRange { name, time });
        }
        // Stable sort at build time keeps same-time events in insertion
        // order — the firing order contract.
        self.events.push(TimelineEvent { time, name });
        Ok(self)
    }

    /// Finishes construction, running the whole-timeline validations.
    ///
    /// # Errors
    /// [`TimelineError::InvalidDuration`] or [`TimelineError::EmptyTimeline`].
    pub fn build(self) -> Result<Timeline, TimelineError> {
        if !self.duration.is_finite() || self.duration <= 0.0 {
            return Err(TimelineError::InvalidDuration {
                duration: self.duration,
            });
        }
        if self.tracks.is_empty() {
            return Err(TimelineError::EmptyTimeline);
        }
        // Stable sort by time: equal-time events keep insertion order.
        let mut events = self.events;
        events.sort_by(|a, b| a.time.total_cmp(&b.time));
        Ok(Timeline {
            duration: self.duration,
            tracks: self.tracks,
            events,
            camera: self.camera,
        })
    }
}

/// Internal: distinguishes "key out of range" from "track invalid".
enum RangeError {
    OutOfRange,
    Track(TrackError),
}

/// Every key must lie in `[0, duration]`; tracks self-validate first.
fn validate_track_range(track: &Track, duration: f32) -> Result<(), RangeError> {
    macro_rules! check {
        ($keys:expr, $time_of:expr) => {{
            // Delegate emptiness/monotonicity/non-finiteness checks to
            // a direct scan here for range + finiteness.
            if $keys.is_empty() {
                return Err(RangeError::Track(TrackError::EmptyTrack));
            }
            for k in $keys.iter() {
                let t = $time_of(k);
                if !t.is_finite() || t < 0.0 || t > duration {
                    return Err(RangeError::OutOfRange);
                }
            }
        }};
    }
    match track {
        Track::Float(t) => check!(t.keys(), |k: &crate::track::FloatKey| k.time),
        Track::Vec3(t) => check!(t.keys(), |k: &crate::track::Vec3Key| k.time),
        Track::Quat(t) => check!(t.keys(), |k: &crate::track::QuatKey| k.time),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{FloatKey, Interpolation, Vec3Key};
    use gens_math::vec3;

    fn vec3_track(keys: &[(f32, [f32; 3])]) -> Track {
        Track::Vec3(
            Vec3Track::new(
                keys.iter()
                    .map(|&(t, v)| Vec3Key {
                        time: t,
                        value: vec3(v[0], v[1], v[2]),
                    })
                    .collect(),
                Interpolation::Linear,
            )
            .expect("valid vec3 track"),
        )
    }

    #[test]
    fn builds_a_valid_timeline() {
        let tl = Timeline::builder(4.0)
            .vec3_track(
                "hero",
                Channel::Translation,
                match vec3_track(&[(0.0, [0.0, 0.0, 0.0]), (4.0, [4.0, 0.0, 0.0])]) {
                    Track::Vec3(t) => t,
                    _ => unreachable!(),
                },
            )
            .expect("track ok")
            .float_track(
                "camera",
                Channel::Fov,
                FloatTrack::new(
                    vec![
                        FloatKey {
                            time: 0.0,
                            value: 60.0,
                        },
                        FloatKey {
                            time: 4.0,
                            value: 40.0,
                        },
                    ],
                    Interpolation::Linear,
                )
                .unwrap(),
            )
            .expect("track ok")
            .event(2.0, "door-opens")
            .expect("event ok")
            .build()
            .expect("timeline ok");
        assert_eq!(tl.duration(), 4.0);
        assert_eq!(tl.events().len(), 1);
        assert_eq!(tl.tracks().count(), 2);
    }

    #[test]
    fn rejects_invalid_duration_and_empty() {
        assert_eq!(
            Timeline::builder(0.0).build(),
            Err(TimelineError::InvalidDuration { duration: 0.0 })
        );
        // NaN never equals NaN — assert by pattern.
        assert!(matches!(
            Timeline::builder(f32::NAN).build(),
            Err(TimelineError::InvalidDuration { duration }) if duration.is_nan()
        ));
        let b = Timeline::builder(1.0).event(0.5, "lone").expect("event ok");
        assert_eq!(b.build(), Err(TimelineError::EmptyTimeline));
    }

    #[test]
    fn rejects_duplicate_and_kind_mismatch() {
        let b = Timeline::builder(2.0)
            .float_track(
                "cam",
                Channel::Fov,
                FloatTrack::new(
                    vec![FloatKey {
                        time: 0.0,
                        value: 60.0,
                    }],
                    Interpolation::Linear,
                )
                .unwrap(),
            )
            .expect("first ok");
        assert_eq!(
            b.float_track(
                "cam",
                Channel::Fov,
                FloatTrack::new(
                    vec![FloatKey {
                        time: 1.0,
                        value: 50.0,
                    }],
                    Interpolation::Linear,
                )
                .unwrap(),
            )
            .unwrap_err(),
            TimelineError::DuplicateTrack {
                target: "cam".to_string(),
                channel: Channel::Fov,
            }
        );
        // A Float track on the Translation channel is a kind mismatch.
        let b2 = Timeline::builder(2.0);
        assert!(matches!(
            b2.float_track(
                "hero",
                Channel::Translation,
                FloatTrack::new(
                    vec![FloatKey {
                        time: 0.0,
                        value: 0.0
                    }],
                    Interpolation::Linear,
                )
                .unwrap(),
            ),
            Err(TimelineError::KindMismatch { .. })
        ));
    }

    #[test]
    fn rejects_keys_outside_duration() {
        let b = Timeline::builder(1.0);
        let result = b.vec3_track(
            "hero",
            Channel::Translation,
            match vec3_track(&[(0.0, [0.0; 3]), (2.0, [1.0; 3])]) {
                Track::Vec3(t) => t,
                _ => unreachable!(),
            },
        );
        assert_eq!(
            result.unwrap_err(),
            TimelineError::KeyOutOfRange {
                target: "hero".to_string(),
                channel: Channel::Translation,
            }
        );
    }

    #[test]
    fn rejects_events_outside_or_zero() {
        let b = Timeline::builder(1.0);
        assert_eq!(
            b.event(0.0, "at-zero").unwrap_err(),
            TimelineError::EventOutOfRange {
                name: "at-zero".to_string(),
                time: 0.0,
            }
        );
        let b = Timeline::builder(1.0);
        assert_eq!(
            b.event(1.5, "past-end").unwrap_err(),
            TimelineError::EventOutOfRange {
                name: "past-end".to_string(),
                time: 1.5,
            }
        );
    }

    #[test]
    fn events_sort_by_time_stably() {
        let tl = Timeline::builder(3.0)
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
            .event(2.0, "second-inserted-first")
            .expect("ok")
            .event(1.0, "first-inserted-second")
            .expect("ok")
            .event(2.0, "same-time-later")
            .expect("ok")
            .build()
            .expect("ok");
        let names: Vec<&str> = tl.events().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "first-inserted-second",
                "second-inserted-first",
                "same-time-later"
            ]
        );
    }

    #[test]
    fn events_between_is_half_open() {
        let tl = Timeline::builder(3.0)
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
            .event(1.0, "a")
            .expect("ok")
            .event(2.0, "b")
            .expect("ok")
            .build()
            .expect("ok");
        // (0, 1] fires 'a'; (1, 1] fires nothing; (0, 2] fires both.
        assert_eq!(tl.events_between(0.0, 1.0).len(), 1);
        assert_eq!(tl.events_between(1.0, 1.0).len(), 0);
        assert_eq!(tl.events_between(0.0, 2.0).len(), 2);
    }

    #[test]
    fn sample_all_covers_every_track_in_order() {
        let tl = Timeline::builder(2.0)
            .vec3_track(
                "b-target",
                Channel::Translation,
                match vec3_track(&[(0.0, [0.0; 3]), (2.0, [2.0, 0.0, 0.0])]) {
                    Track::Vec3(t) => t,
                    _ => unreachable!(),
                },
            )
            .expect("ok")
            .float_track(
                "a-target",
                Channel::Fov,
                FloatTrack::new(
                    vec![
                        FloatKey {
                            time: 0.0,
                            value: 60.0,
                        },
                        FloatKey {
                            time: 2.0,
                            value: 40.0,
                        },
                    ],
                    Interpolation::Linear,
                )
                .unwrap(),
            )
            .expect("ok")
            .build()
            .expect("ok");
        let sampled = tl.sample_all(1.0);
        // BTreeMap order: target "a-target" sorts before "b-target".
        assert_eq!(sampled.len(), 2);
        assert_eq!(sampled[0].0.target, "a-target");
        assert_eq!(sampled[0].1, SampledValue::Float(50.0));
        assert_eq!(sampled[1].0.target, "b-target");
        assert_eq!(sampled[1].1, SampledValue::Vec3(vec3(1.0, 0.0, 0.0)));
    }

    #[test]
    fn camera_intent_round_trips() {
        let tl = Timeline::builder(2.0)
            .float_track(
                "cam",
                Channel::Fov,
                FloatTrack::new(
                    vec![FloatKey {
                        time: 0.0,
                        value: 60.0,
                    }],
                    Interpolation::Linear,
                )
                .unwrap(),
            )
            .expect("ok")
            .camera(CameraCut {
                target: "cam".to_string(),
                hard_cut: true,
                blend_in: 0.0,
            })
            .build()
            .expect("ok");
        assert_eq!(
            tl.camera(),
            Some(&CameraCut {
                target: "cam".to_string(),
                hard_cut: true,
                blend_in: 0.0,
            })
        );
    }
}
