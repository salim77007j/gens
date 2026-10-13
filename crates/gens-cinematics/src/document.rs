//! The `gens-cine-1` document format (Phase 10, ADR-0033).
//!
//! Cinematic timelines were authored in code only — the Phase 9 handoff
//! named a serialized format as the natural polish. This module is that
//! format, built the way the save format (ADR-0021) and every proof in
//! this repo demands:
//!
//! - **Exact f32 fidelity** — every float travels as its IEEE-754 bit
//!   pattern in 8-digit hex (`"3f7ffffb"`), never as a decimal literal.
//!   A decimal round-trip would corrupt the low bits the determinism
//!   proofs pin (the seek lesson: `60 × (1/60)` is `0.9999997`, bits
//!   `3f7ffffb`, not `1.0`), and decimal parsing is platform-adjacent
//!   behavior we refuse to depend on. Parse yields the exact bits back.
//! - **Validation is the constructors'** — [`from_document`] rebuilds
//!   the timeline through [`FloatTrack`]/[`Vec3Track`]/[`QuatTrack`] and
//!   [`TimelineBuilder`], so every construction rule (empty tracks,
//!   duplicate times, non-finite values, kind-vs-channel match, key
//!   range, event window, duration) applies to loaded data with zero
//!   new validation code to drift. A document is either a valid
//!   [`Timeline`] or a typed [`DocumentError`]; there is no
//!   skip-validation path.
//! - **Canonical byte form** — tracks serialize in the timeline's
//!   deterministic `(target, channel)` order, events in firing order;
//!   two equal timelines serialize to byte-identical documents, so
//!   [`content_hash`] (FNV-1a over the canonical bytes, the heightfield
//!   pattern from `gens-worldgen`) is stable for equal content.
//!
//! The core timeline types stay serde-free on purpose: the wire
//! vocabulary lives here, mirrored field by field, so the format can
//! evolve without the sampling semantics ever caring about JSON.

use crate::timeline::{CameraCut, Channel, Timeline, Track};
use crate::track::{FloatKey, FloatTrack, Interpolation, QuatKey, QuatTrack, Vec3Key, Vec3Track};
use gens_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The format tag every `gens-cine-1` document carries.
pub const FORMAT_TAG: &str = "gens-cine-1";

/// Errors from parsing or rebuilding a `gens-cine-1` document.
#[derive(Debug, Error)]
pub enum DocumentError {
    /// The text is not valid JSON or does not match the document shape.
    #[error("document is not valid gens-cine-1 JSON: {0}")]
    Parse(#[from] serde_json::Error),
    /// The document declares a format tag other than [`FORMAT_TAG`].
    #[error("unknown format tag `{found}` (expected `{FORMAT_TAG}`)")]
    FormatTag {
        /// The tag the document actually carried.
        found: String,
    },
    /// A track channel name is not one of the five known channels.
    #[error("unknown channel `{0}` (expected translation|rotation|scale|fov|fade)")]
    Channel(String),
    /// A track interpolation name is not one of the three known modes.
    #[error("unknown interpolation `{0}` (expected linear|step|smoothstep)")]
    Interpolation(String),
    /// A float bit field is not 8 hex digits.
    #[error("invalid f32 bit field `{0}` (expected 8 hex digits)")]
    FloatBits(String),
    /// A track's key data failed its own constructor validation
    /// (unsorted/duplicate times, empty, non-finite).
    #[error("track data rejected: {0}")]
    TrackData(#[from] crate::track::TrackError),
    /// The document declares a track kind outside `float|vec3|quat`.
    #[error("unknown track kind `{0}` (expected float|vec3|quat)")]
    TrackKind(String),
    /// The timeline constructors rejected the document's content —
    /// the same errors an in-code author would get, by design.
    #[error("timeline rejected by its own constructors: {0}")]
    Timeline(#[from] crate::timeline::TimelineError),
}

/// Serializes an `f32` as its IEEE-754 bit pattern in 8-digit hex.
///
/// Decimal text would round-trip through platform parsing and could
/// not represent NaN payloads distinctly; bits do both jobs exactly.
mod bits {
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serde serialize half: `f32` → `"3f7ffffb"`.
    pub fn serialize<S: Serializer>(value: &f32, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{:08x}", value.to_bits()))
    }

    /// Serde deserialize half: `"3f7ffffb"` → the exact `f32` bits.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() != 8 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(serde::de::Error::custom(format!(
                "invalid f32 bit field `{text}` (expected 8 hex digits)"
            )));
        }
        u32::from_str_radix(&text, 16)
            .map(f32::from_bits)
            .map_err(|_| serde::de::Error::custom(format!("invalid f32 bit field `{text}`")))
    }
}

#[derive(Serialize, Deserialize)]
struct CineDocument {
    format: String,
    #[serde(with = "bits")]
    duration: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    camera: Option<CameraCutDoc>,
    tracks: Vec<TrackDoc>,
    events: Vec<EventDoc>,
}

#[derive(Serialize, Deserialize)]
struct CameraCutDoc {
    target: String,
    hard_cut: bool,
    #[serde(with = "bits")]
    blend_in: f32,
}

#[derive(Serialize, Deserialize)]
struct TrackDoc {
    target: String,
    channel: String,
    #[serde(flatten)]
    data: TrackDataDoc,
}

#[derive(Serialize, Deserialize)]
struct TrackDataDoc {
    kind: String,
    interpolation: String,
    #[serde(default)]
    float_keys: Vec<FloatKeyDoc>,
    #[serde(default)]
    vec3_keys: Vec<Vec3KeyDoc>,
    #[serde(default)]
    quat_keys: Vec<QuatKeyDoc>,
}

#[derive(Serialize, Deserialize)]
struct FloatKeyDoc {
    #[serde(with = "bits")]
    time: f32,
    #[serde(with = "bits")]
    value: f32,
}

#[derive(Serialize, Deserialize)]
struct Vec3KeyDoc {
    #[serde(with = "bits")]
    time: f32,
    value: Vec3Doc,
}

#[derive(Serialize, Deserialize)]
struct QuatKeyDoc {
    #[serde(with = "bits")]
    time: f32,
    value: QuatDoc,
}

#[derive(Serialize, Deserialize)]
struct Vec3Doc {
    #[serde(with = "bits")]
    x: f32,
    #[serde(with = "bits")]
    y: f32,
    #[serde(with = "bits")]
    z: f32,
}

#[derive(Serialize, Deserialize)]
struct QuatDoc {
    #[serde(with = "bits")]
    x: f32,
    #[serde(with = "bits")]
    y: f32,
    #[serde(with = "bits")]
    z: f32,
    #[serde(with = "bits")]
    w: f32,
}

#[derive(Serialize, Deserialize)]
struct EventDoc {
    #[serde(with = "bits")]
    time: f32,
    name: String,
}

fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Translation => "translation",
        Channel::Rotation => "rotation",
        Channel::Scale => "scale",
        Channel::Fov => "fov",
        Channel::Fade => "fade",
    }
}

fn parse_channel(name: &str) -> Result<Channel, DocumentError> {
    match name {
        "translation" => Ok(Channel::Translation),
        "rotation" => Ok(Channel::Rotation),
        "scale" => Ok(Channel::Scale),
        "fov" => Ok(Channel::Fov),
        "fade" => Ok(Channel::Fade),
        other => Err(DocumentError::Channel(other.to_string())),
    }
}

fn interpolation_name(interpolation: Interpolation) -> &'static str {
    match interpolation {
        Interpolation::Linear => "linear",
        Interpolation::Step => "step",
        Interpolation::Smoothstep => "smoothstep",
    }
}

fn parse_interpolation(name: &str) -> Result<Interpolation, DocumentError> {
    match name {
        "linear" => Ok(Interpolation::Linear),
        "step" => Ok(Interpolation::Step),
        "smoothstep" => Ok(Interpolation::Smoothstep),
        other => Err(DocumentError::Interpolation(other.to_string())),
    }
}

/// Serializes a timeline to the canonical `gens-cine-1` document
/// (compact JSON — the byte form [`content_hash`] runs over).
///
/// Track order is the timeline's deterministic `(target, channel)`
/// order; events are in firing order. Two equal timelines produce
/// byte-identical documents.
///
/// ```
/// use gens_cinematics::document::{to_canonical_json, from_document};
/// use gens_cinematics::{Channel, Timeline};
///
/// let timeline = Timeline::builder(2.0)
///     .float_track("cam", Channel::Fov,
///         gens_cinematics::FloatTrack::new(
///             vec![gens_cinematics::FloatKey { time: 0.0, value: 60.0 }],
///             gens_cinematics::Interpolation::Linear).unwrap())
///     .unwrap()
///     .build().unwrap();
/// let json = to_canonical_json(&timeline);
/// assert_eq!(from_document(&json).unwrap(), timeline);
/// ```
pub fn to_canonical_json(timeline: &Timeline) -> String {
    let doc = document_of(timeline);
    serde_json::to_string(&doc).expect("gens-cine-1 document serialization cannot fail")
}

/// Serializes a timeline to the pretty (human-diffable) `gens-cine-1`
/// form. Byte-different from [`to_canonical_json`] but semantically
/// identical — evidence files embed this shape.
pub fn to_pretty_json(timeline: &Timeline) -> String {
    let doc = document_of(timeline);
    serde_json::to_string_pretty(&doc).expect("gens-cine-1 document serialization cannot fail")
}

fn document_of(timeline: &Timeline) -> CineDocument {
    let tracks = timeline
        .tracks()
        .map(|(address, track)| TrackDoc {
            target: address.target.clone(),
            channel: channel_name(address.channel).to_string(),
            data: track_data_of(track),
        })
        .collect();
    let events = timeline
        .events()
        .iter()
        .map(|event| EventDoc {
            time: event.time,
            name: event.name.clone(),
        })
        .collect();
    CineDocument {
        format: FORMAT_TAG.to_string(),
        duration: timeline.duration(),
        camera: timeline.camera().map(|cut| CameraCutDoc {
            target: cut.target.clone(),
            hard_cut: cut.hard_cut,
            blend_in: cut.blend_in,
        }),
        tracks,
        events,
    }
}

fn track_data_of(track: &Track) -> TrackDataDoc {
    let (kind, interpolation) = match track {
        Track::Float(t) => ("float", t.interpolation()),
        Track::Vec3(t) => ("vec3", t.interpolation()),
        Track::Quat(t) => ("quat", t.interpolation()),
    };
    let mut data = TrackDataDoc {
        kind: kind.to_string(),
        interpolation: interpolation_name(interpolation).to_string(),
        float_keys: Vec::new(),
        vec3_keys: Vec::new(),
        quat_keys: Vec::new(),
    };
    match track {
        Track::Float(t) => {
            data.float_keys = t
                .keys()
                .iter()
                .map(|k| FloatKeyDoc {
                    time: k.time,
                    value: k.value,
                })
                .collect();
        }
        Track::Vec3(t) => {
            data.vec3_keys = t
                .keys()
                .iter()
                .map(|k| Vec3KeyDoc {
                    time: k.time,
                    value: vec3_doc_of(k.value),
                })
                .collect();
        }
        Track::Quat(t) => {
            data.quat_keys = t
                .keys()
                .iter()
                .map(|k| QuatKeyDoc {
                    time: k.time,
                    value: quat_doc_of(k.value),
                })
                .collect();
        }
    }
    data
}

fn vec3_doc_of(value: Vec3) -> Vec3Doc {
    Vec3Doc {
        x: value.x,
        y: value.y,
        z: value.z,
    }
}

fn quat_doc_of(value: Quat) -> QuatDoc {
    QuatDoc {
        x: value.x,
        y: value.y,
        z: value.z,
        w: value.w,
    }
}

/// Parses and rebuilds a timeline from a `gens-cine-1` document.
///
/// Validation is routed through the timeline constructors, so every
/// rule an in-code author faces applies identically to loaded data —
/// the document module adds no validation of its own and can never
/// drift from it.
pub fn from_document(text: &str) -> Result<Timeline, DocumentError> {
    let doc: CineDocument = serde_json::from_str(text)?;
    if doc.format != FORMAT_TAG {
        return Err(DocumentError::FormatTag { found: doc.format });
    }
    let mut builder = Timeline::builder(doc.duration);
    if let Some(camera) = doc.camera {
        builder = builder.camera(CameraCut {
            target: camera.target,
            hard_cut: camera.hard_cut,
            blend_in: camera.blend_in,
        });
    }
    for track_doc in doc.tracks {
        let channel = parse_channel(&track_doc.channel)?;
        let track = build_track(&track_doc.data).map_err(|err| match err {
            DocumentError::TrackData(source) => {
                DocumentError::Timeline(crate::timeline::TimelineError::InvalidTrack {
                    target: track_doc.target.clone(),
                    channel,
                    source,
                })
            }
            other => other,
        })?;
        builder = builder.track(track_doc.target, channel, track)?;
    }
    for event in doc.events {
        builder = builder.event(event.time, event.name)?;
    }
    Ok(builder.build()?)
}

fn build_track(data: &TrackDataDoc) -> Result<Track, DocumentError> {
    let interpolation = parse_interpolation(&data.interpolation)?;
    match data.kind.as_str() {
        "float" => {
            let keys = data
                .float_keys
                .iter()
                .map(|k| FloatKey {
                    time: k.time,
                    value: k.value,
                })
                .collect::<Vec<_>>();
            Ok(Track::Float(
                FloatTrack::new(keys, interpolation).map_err(DocumentError::TrackData)?,
            ))
        }
        "vec3" => {
            let keys = data
                .vec3_keys
                .iter()
                .map(|k| Vec3Key {
                    time: k.time,
                    value: Vec3::new(k.value.x, k.value.y, k.value.z),
                })
                .collect::<Vec<_>>();
            Ok(Track::Vec3(
                Vec3Track::new(keys, interpolation).map_err(DocumentError::TrackData)?,
            ))
        }
        "quat" => {
            let keys = data
                .quat_keys
                .iter()
                .map(|k| QuatKey {
                    time: k.time,
                    value: Quat::from_xyzw(k.value.x, k.value.y, k.value.z, k.value.w),
                })
                .collect::<Vec<_>>();
            Ok(Track::Quat(
                QuatTrack::new(keys, interpolation).map_err(DocumentError::TrackData)?,
            ))
        }
        other => Err(DocumentError::TrackKind(other.to_string())),
    }
}

/// The FNV-1a content hash over the canonical document bytes —
/// the same hash convention `gens-worldgen` uses for heightfields.
///
/// Equal timelines hash equal; any change to a duration, key, event or
/// camera intent changes the hash. Advisory by construction: it pins
/// content identity, not quality.
pub fn content_hash(timeline: &Timeline) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in to_canonical_json(timeline).as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::TrackError;

    /// The honest f32 signature this repo pins everywhere: 60 × (1/60)
    /// accumulates to `0.9999997`, bits `3f7ffffb` — not `1.0`.
    const ACCUMULATED_60_STEPS: f32 = 0.999_999_7;

    /// √2/2 as an f32 literal (the f64→f32 rounding, spelled out).
    const HALF_SQRT2: f32 = 0.707_106_77;

    fn sample_timeline() -> Timeline {
        let float_track = FloatTrack::new(
            vec![
                FloatKey {
                    time: 0.0,
                    value: 60.0,
                },
                FloatKey {
                    time: ACCUMULATED_60_STEPS,
                    value: 42.5,
                },
            ],
            Interpolation::Smoothstep,
        )
        .unwrap();
        let vec3_track = Vec3Track::new(
            vec![
                Vec3Key {
                    time: 0.0,
                    value: Vec3::new(0.0, 1.5, -2.0),
                },
                Vec3Key {
                    time: ACCUMULATED_60_STEPS,
                    value: Vec3::new(3.0, 1.5, 4.0),
                },
            ],
            Interpolation::Linear,
        )
        .unwrap();
        let quat_track = QuatTrack::new(
            vec![
                QuatKey {
                    time: 0.0,
                    value: Quat::from_xyzw(0.0, 0.0, 0.0, 1.0),
                },
                QuatKey {
                    time: 0.5,
                    value: Quat::from_xyzw(0.0, HALF_SQRT2, 0.0, HALF_SQRT2),
                },
            ],
            Interpolation::Step,
        )
        .unwrap();
        Timeline::builder(ACCUMULATED_60_STEPS)
            .camera(CameraCut {
                target: "cam".to_string(),
                hard_cut: false,
                blend_in: 0.25,
            })
            .float_track("cam", Channel::Fov, float_track)
            .unwrap()
            .vec3_track("hero", Channel::Translation, vec3_track)
            .unwrap()
            .quat_track("hero", Channel::Rotation, quat_track)
            .unwrap()
            .event(0.5, "impact")
            .unwrap()
            .build()
            .unwrap()
    }

    #[test]
    fn roundtrip_preserves_every_bit() {
        let timeline = sample_timeline();
        let json = to_canonical_json(&timeline);
        let loaded = from_document(&json).unwrap();
        assert_eq!(loaded, timeline);
        // Pin the honest bits through the whole wire path: the duration
        // is the accumulated 60-step value, not a nearby literal.
        assert_eq!(
            format!("{:08x}", loaded.duration().to_bits()),
            "3f7ffffb",
            "duration bits must survive the document byte-exactly"
        );
        let (_, fov) = loaded.tracks().next().unwrap();
        match fov {
            Track::Float(t) => {
                assert_eq!(format!("{:08x}", t.keys()[1].time.to_bits()), "3f7ffffb")
            }
            other => panic!("expected the float track first, got {other:?}"),
        }
    }

    #[test]
    fn canonical_form_and_hash_are_stable() {
        let a = sample_timeline();
        let b = sample_timeline();
        assert_eq!(to_canonical_json(&a), to_canonical_json(&b));
        assert_eq!(content_hash(&a), content_hash(&b));
        // One key value one ULP off must move the hash.
        let drifted = Timeline::builder(ACCUMULATED_60_STEPS)
            .float_track(
                "cam",
                Channel::Fov,
                FloatTrack::new(
                    vec![
                        FloatKey {
                            time: 0.0,
                            value: 60.0,
                        },
                        FloatKey {
                            time: ACCUMULATED_60_STEPS,
                            value: 42.500_004,
                        },
                    ],
                    Interpolation::Smoothstep,
                )
                .unwrap(),
            )
            .unwrap()
            .build()
            .unwrap();
        assert_ne!(content_hash(&a), content_hash(&drifted));
    }

    #[test]
    fn pretty_form_roundtrips_equally() {
        let timeline = sample_timeline();
        let pretty = to_pretty_json(&timeline);
        assert!(pretty.contains(FORMAT_TAG));
        assert_eq!(from_document(&pretty).unwrap(), timeline);
    }

    #[test]
    fn format_tag_is_enforced() {
        let timeline = sample_timeline();
        let json = to_canonical_json(&timeline).replace(FORMAT_TAG, "gens-cine-0");
        match from_document(&json) {
            Err(DocumentError::FormatTag { found }) => assert_eq!(found, "gens-cine-0"),
            other => panic!("expected FormatTag rejection, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_event_outside_window() {
        // t = 0 is outside the legal (0, duration] event window — the
        // constructor rejects it, so the document must be rejected too.
        let doc = r#"{
            "format": "gens-cine-1",
            "duration": "3f7ffffb",
            "tracks": [{"target": "cam", "channel": "fov", "kind": "float",
                        "interpolation": "linear",
                        "float_keys": [{"time": "00000000", "value": "42700000"}]}],
            "events": [{"time": "00000000", "name": "never"}]
        }"#;
        match from_document(doc) {
            Err(DocumentError::Timeline(crate::timeline::TimelineError::EventOutOfRange {
                ..
            })) => {}
            other => panic!("expected EventOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_duplicate_key_times() {
        let doc = r#"{
            "format": "gens-cine-1",
            "duration": "3f7ffffb",
            "tracks": [{"target": "cam", "channel": "fov", "kind": "float",
                        "interpolation": "linear",
                        "float_keys": [
                            {"time": "00000000", "value": "42700000"},
                            {"time": "00000000", "value": "42c80000"}]}],
            "events": []
        }"#;
        match from_document(doc) {
            Err(DocumentError::Timeline(crate::timeline::TimelineError::InvalidTrack {
                source: TrackError::UnsortedKeys { .. },
                ..
            })) => {}
            other => panic!("expected InvalidTrack(UnsortedKeys), got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_kind_mismatch() {
        // fov is a Float channel; a quat track on it is a construction
        // error in code, so it is one in documents too.
        let doc = r#"{
            "format": "gens-cine-1",
            "duration": "3f7ffffb",
            "tracks": [{"target": "cam", "channel": "fov", "kind": "quat",
                        "interpolation": "linear",
                        "quat_keys": [{"time": "00000000",
                                       "value": {"x": "00000000", "y": "00000000",
                                                 "z": "00000000", "w": "3f800000"}}]}],
            "events": []
        }"#;
        match from_document(doc) {
            Err(DocumentError::Timeline(crate::timeline::TimelineError::KindMismatch {
                ..
            })) => {}
            other => panic!("expected KindMismatch, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_non_finite_key_bits() {
        // +inf bits (7f800000) decode fine at the wire layer — and are
        // then rejected by the same constructor rule that rejects an
        // in-code NaN. Silent poisoning is impossible by construction.
        let doc = r#"{
            "format": "gens-cine-1",
            "duration": "3f7ffffb",
            "tracks": [{"target": "cam", "channel": "fov", "kind": "float",
                        "interpolation": "linear",
                        "float_keys": [{"time": "00000000", "value": "7f800000"}]}],
            "events": []
        }"#;
        match from_document(doc) {
            Err(DocumentError::Timeline(crate::timeline::TimelineError::InvalidTrack {
                ..
            })) => {}
            other => panic!("expected InvalidTrack for +inf, got {other:?}"),
        }
    }

    #[test]
    fn malformed_bits_are_typed_errors() {
        let doc = r#"{
            "format": "gens-cine-1",
            "duration": "not-hex",
            "tracks": [], "events": []
        }"#;
        match from_document(doc) {
            Err(DocumentError::Parse(_)) => {}
            other => panic!("expected Parse rejection for bad bits, got {other:?}"),
        }
    }
}
