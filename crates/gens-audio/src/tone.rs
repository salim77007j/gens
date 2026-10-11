//! Test/demo sound synthesis: deterministic tones with no binary assets.
//!
//! CI has no audio files and no devices; every audio test builds its sounds
//! from raw samples here. The same helpers work for engine demos that want
//! procedurally generated audio without an asset pipeline.

use kira::Frame;
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};

/// A monophonic sine tone as stereo `StaticSoundData`.
///
/// `duration_secs` frames are generated at `sample_rate`; both channels
/// carry the same wave so stereo contracts stay trivially checkable.
#[must_use]
pub fn sine(duration_secs: f32, freq_hz: f32, sample_rate: u32, amplitude: f32) -> StaticSoundData {
    let frames = (duration_secs * sample_rate as f32).round() as usize;
    let data: Vec<Frame> = (0..frames)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let sample = amplitude * (2.0 * std::f32::consts::PI * freq_hz * t).sin();
            Frame {
                left: sample,
                right: sample,
            }
        })
        .collect();
    StaticSoundData {
        sample_rate,
        frames: data.into(),
        settings: StaticSoundSettings::default(),
        slice: None,
    }
}

/// `duration_secs` of digital silence (zero samples), for scheduling tests.
#[must_use]
pub fn silence(duration_secs: f32, sample_rate: u32) -> StaticSoundData {
    let frames = (duration_secs * sample_rate as f32).round() as usize;
    StaticSoundData {
        sample_rate,
        frames: vec![
            Frame {
                left: 0.0,
                right: 0.0,
            };
            frames
        ]
        .into(),
        settings: StaticSoundSettings::default(),
        slice: None,
    }
}

/// True-RMS (root mean square) of the left channel — a coarse but honest
/// signal-level metric for tests: silence ≈ 0, a full-scale sine ≈ A/√2.
#[must_use]
pub fn rms_left(data: &StaticSoundData) -> f32 {
    if data.frames.is_empty() {
        return 0.0;
    }
    let sum: f32 = data.frames.iter().map(|f| f.left * f.left).sum();
    (sum / data.frames.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_has_exact_frame_count_and_duration() {
        let data = sine(0.5, 440.0, 44_100, 0.8);
        assert_eq!(data.frames.len(), 22_050);
        let duration = data.frames.len() as f64 / data.sample_rate as f64;
        assert!((duration - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sine_amplitude_follows_rms_contract() {
        let full = rms_left(&sine(0.1, 440.0, 44_100, 1.0));
        // RMS of a full-scale sine is 1/√2 ≈ 0.7071.
        assert!(
            (full - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "rms {full}"
        );
        let quiet = rms_left(&sine(0.1, 440.0, 44_100, 0.5));
        assert!((quiet - full / 2.0).abs() < 1e-3, "rms scales linearly");
        let silent = rms_left(&silence(0.01, 44_100));
        assert_eq!(silent, 0.0);
    }

    #[test]
    fn sine_first_sample_is_zero_and_rises() {
        let data = sine(0.1, 100.0, 1000, 1.0);
        assert_eq!(data.frames[0].left, 0.0, "sin(0) = 0");
        assert!(data.frames[1].left > 0.0, "phase rises for positive freq");
        assert_eq!(data.frames[0].left, data.frames[0].right, "stereo twin");
    }
}
