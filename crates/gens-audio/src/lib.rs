//! Audio playback and mixing on kira (Phase 3, ADR-0014).
//!
//! Phase 3 delivers the **headless audio core**: an [`AudioEngine`] over
//! kira's `MockBackend`, which runs the real mixer/renderer in-process with
//! no audio device. CI (and any device-less machine) verifies the full
//! playback state machine — play/pause/resume/stop, positions, looping —
//! deterministically, once per [`AudioEngine::process`] tick.
//!
//! The realtime output backend (kira's cpal feature) is intentionally left
//! unpinned: it needs ALSA headers on Linux build machines, and device
//! output belongs with the windowing phase. Enabling it later is a
//! feature-gated addition, not a redesign (ADR-0014). The raw kira crate is
//! reachable as [`backend`], mirroring the `gens-ecs::ecs` convention.
//!
//! Sample-level output capture is not exposed by kira's mock backend
//! (its output buffer is private), so the honest verification surface here
//! is scheduling/state semantics plus the synthesized-data contracts in
//! [`tone`] — not DSP samples.

pub mod tone;

use thiserror::Error;

/// The underlying kira crate (0.12.x), re-exported for advanced use.
///
/// Engine code should prefer the facade; see ADR-0014.
pub use kira as backend;

pub use kira::sound::PlaybackState;
pub use kira::sound::static_sound::StaticSoundData;

use kira::backend::mock::{MockBackend, MockBackendSettings};
use kira::sound::static_sound::StaticSoundHandle;
use kira::{AudioManager, AudioManagerSettings, Tween};

/// Errors raised by the audio facade.
#[derive(Debug, Error)]
pub enum AudioError {
    /// The sound could not be scheduled for playback.
    #[error("playback request failed: {0}")]
    Play(String),
    /// The backend could not be initialized.
    #[error("audio backend init failed: {0}")]
    Backend(String),
}

/// The headless audio engine: kira's mixer driven tick-by-tick.
///
/// Every [`AudioEngine::process`] call advances the real renderer by one
/// internal buffer, deterministically — the audio equivalent of one fixed
/// simulation step.
pub struct AudioEngine {
    manager: AudioManager<MockBackend>,
    sample_rate: u32,
}

impl AudioEngine {
    /// Default sample rate for the headless mixer (44.1 kHz, CD standard).
    pub const DEFAULT_SAMPLE_RATE: u32 = 44_100;

    /// Creates an engine at [`AudioEngine::DEFAULT_SAMPLE_RATE`].
    pub fn new() -> Result<AudioEngine, AudioError> {
        AudioEngine::with_sample_rate(Self::DEFAULT_SAMPLE_RATE)
    }

    /// Creates an engine at `sample_rate` Hz.
    ///
    /// The mock backend never fails to initialize; the `Result` keeps the
    /// signature backend-stable for the realtime backend later (ADR-0014).
    pub fn with_sample_rate(sample_rate: u32) -> Result<AudioEngine, AudioError> {
        let manager = AudioManager::<MockBackend>::new(AudioManagerSettings {
            backend_settings: MockBackendSettings { sample_rate },
            ..Default::default()
        })
        .map_err(|e| AudioError::Backend(format!("{e:?}")))?;
        Ok(AudioEngine {
            manager,
            sample_rate,
        })
    }

    /// The mixer's sample rate in Hz.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Starts playing `sound` and returns a handle to control it.
    pub fn play(&mut self, sound: StaticSoundData) -> Result<SoundHandle, AudioError> {
        self.manager
            .play(sound)
            .map(SoundHandle)
            .map_err(|e| AudioError::Play(e.to_string()))
    }

    /// Advances the headless mixer by one internal buffer.
    ///
    /// Command queue drains, playback states update, samples render (into
    /// the backend's private buffer). Call once per frame or per test tick.
    pub fn process(&mut self) {
        let backend = self.manager.backend_mut();
        backend.on_start_processing();
        backend.process();
    }

    /// Convenience: advances the mixer `ticks` times.
    pub fn process_n(&mut self, ticks: usize) {
        for _ in 0..ticks {
            self.process();
        }
    }
}

impl Default for AudioEngine {
    /// Panics only if the default sample rate engine cannot initialize
    /// (unreachable with the mock backend; kept for `Default` ergonomics).
    fn default() -> Self {
        AudioEngine::new().expect("mock audio backend cannot fail to init")
    }
}

/// A handle to one playing sound.
///
/// Thin wrapper over kira's `StaticSoundHandle`; transitions use kira's
/// default 10 ms tween, so a state change settles within a few
/// [`AudioEngine::process`] ticks (measured: <=4 at 44.1 kHz / 128-frame
/// buffers).
#[derive(Debug)]
pub struct SoundHandle(StaticSoundHandle);

impl SoundHandle {
    /// Current playback state (updated by [`AudioEngine::process`]).
    #[must_use]
    pub fn state(&self) -> PlaybackState {
        self.0.state()
    }

    /// Whether the sound is currently producing audio.
    #[must_use]
    pub fn is_advancing(&self) -> bool {
        self.0.state().is_advancing()
    }

    /// Playback position in seconds.
    #[must_use]
    pub fn position(&self) -> f64 {
        self.0.position()
    }

    /// Pauses playback (default 10 ms fade-out tween).
    pub fn pause(&mut self) {
        self.0.pause(Tween::default());
    }

    /// Resumes a paused sound (default 10 ms fade-in tween).
    pub fn resume(&mut self) {
        self.0.resume(Tween::default());
    }

    /// Stops the sound permanently (default 10 ms fade-out); the handle
    /// reports [`PlaybackState::Stopped`] afterwards and the sound is
    /// reclaimed by the mixer.
    pub fn stop(&mut self) {
        self.0.stop(Tween::default());
    }

    /// Changes the loop region while playing (see kira's `Region`).
    pub fn set_loop_region(&mut self, region: impl Into<kira::sound::Region>) {
        self.0.set_loop_region(region);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tone::{silence, sine};

    /// Frames the mock mixer renders per [`AudioEngine::process`] tick
    /// (kira's default internal buffer size), at 44.1 kHz.
    const TICK_SECS: f64 = 128.0 / 44_100.0;

    /// kira's default tween is 10 ms; 8 ticks (~23 ms of audio) settle any
    /// default-tween state transition with margin.
    fn settle(engine: &mut AudioEngine) {
        engine.process_n(8);
    }

    fn engine() -> AudioEngine {
        AudioEngine::with_sample_rate(44_100).expect("mock backend init")
    }

    #[test]
    fn new_sound_is_playing_and_advances_with_process() {
        let mut engine = engine();
        let handle = engine.play(silence(10.0, 44_100)).expect("play");
        assert_eq!(handle.state(), PlaybackState::Playing);

        settle(&mut engine);
        let p1 = handle.position();
        assert!(p1 > 0.0, "position must advance after process, got {p1}");
        settle(&mut engine);
        let p2 = handle.position();
        assert!(p2 > p1, "position must keep advancing: {p1} -> {p2}");
        // Each tick advances exactly one internal buffer (128 frames).
        let delta = p2 - p1;
        let expected = TICK_SECS * 8.0;
        assert!(
            (delta - expected).abs() < 1e-6,
            "8 ticks must advance 8 buffers: {delta} vs {expected}"
        );
    }

    #[test]
    fn pause_resume_stop_state_machine() {
        let mut engine = engine();
        let mut handle = engine.play(silence(10.0, 44_100)).expect("play");
        settle(&mut engine);

        handle.pause();
        settle(&mut engine);
        assert_eq!(handle.state(), PlaybackState::Paused);
        assert!(!handle.is_advancing());
        let frozen = handle.position();

        // Paused sounds must not advance.
        settle(&mut engine);
        assert_eq!(handle.position(), frozen, "paused position is frozen");

        handle.resume();
        settle(&mut engine);
        assert_eq!(handle.state(), PlaybackState::Playing);
        assert!(handle.is_advancing());
        let resumed = handle.position();
        assert!(resumed > frozen, "resume must continue from frozen point");

        handle.stop();
        settle(&mut engine);
        assert_eq!(handle.state(), PlaybackState::Stopped);
        assert!(!handle.is_advancing());
    }

    #[test]
    fn stopped_sounds_stay_stopped_but_others_keep_playing() {
        let mut engine = engine();
        let mut a = engine.play(silence(10.0, 44_100)).expect("play a");
        let b = engine.play(silence(10.0, 44_100)).expect("play b");
        settle(&mut engine);

        a.stop();
        settle(&mut engine);
        assert_eq!(a.state(), PlaybackState::Stopped);
        assert_eq!(b.state(), PlaybackState::Playing, "independent handles");

        // A stopped handle stays stopped even as the mixer keeps running.
        settle(&mut engine);
        assert_eq!(a.state(), PlaybackState::Stopped);
    }

    #[test]
    fn looping_sound_never_stops() {
        let mut engine = engine();
        // 0.25 s tone looping over its full region: 256 ticks (~0.74 s of
        // audio) must not exhaust it.
        let mut data = sine(0.25, 440.0, 44_100, 0.5);
        data.settings = data.settings.loop_region(0.0..0.25);
        let handle = engine.play(data).expect("play looped");
        engine.process_n(256);
        assert_eq!(
            handle.state(),
            PlaybackState::Playing,
            "looped sound must survive past its own length"
        );
        // Position wrapped back inside the loop region.
        assert!(
            handle.position() < 0.25,
            "wrapped position, got {}",
            handle.position()
        );
    }

    #[test]
    fn one_shot_sound_stops_at_end() {
        let mut engine = engine();
        let handle = engine.play(silence(0.05, 44_100)).expect("play");
        // 0.05 s = 2205 frames = ~18 buffer ticks; run 64.
        engine.process_n(64);
        assert_eq!(handle.state(), PlaybackState::Stopped, "one-shot ends");
    }

    #[test]
    fn per_sound_volume_and_panning_settings_are_accepted() {
        let mut engine = engine();
        let mut data = silence(1.0, 44_100);
        data.settings = data.settings.volume(kira::Decibels(-6.0)).panning(0.25);
        let handle = engine.play(data).expect("play with settings");
        settle(&mut engine);
        assert_eq!(handle.state(), PlaybackState::Playing);
    }

    #[test]
    fn default_sample_rate_contract() {
        let engine = engine();
        assert_eq!(engine.sample_rate(), 44_100);
        assert_eq!(AudioEngine::DEFAULT_SAMPLE_RATE, 44_100);
    }
}
