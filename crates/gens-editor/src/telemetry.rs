//! Telemetry: the bridge between the observation-only profiler and the
//! editor panels (P8-6).
//!
//! The app owns the [`Profiler`] and feeds each frame's spans into the
//! shell via [`Telemetry::update_from`]; the Profiler/Performance tabs and
//! the status bar render the aggregation. Everything here is
//! observation-only (ADR-0027): telemetry reads wall-clock sidecar data
//! and writes nothing the simulation can see, so the byte-identical
//! replay contract is untouched — and profiled/unprofiled replays stay
//! byte-identical (proven per integration, see `profile_hello`).
//!
//! Aggregation is a rolling window over the last N recorded frames,
//! median = sorted[len/2] (the gens-profiler convention), max = worst
//! observation in the window.

use gens_profiler::Profiler;
use serde::Serialize;
use std::collections::BTreeMap;

/// One phase's rolling timing summary (milliseconds).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhaseTimings {
    /// Span name (`"scripts"`, `"physics"`, `"viewport"`, ...).
    pub name: String,
    /// Median duration over the window.
    pub median_ms: f32,
    /// Worst duration over the window.
    pub max_ms: f32,
    /// Share of frame wall time across the window (0..1).
    pub share: f32,
}

/// Rolling frame telemetry, recomputed from profiler history.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Telemetry {
    /// Per-phase summaries, sorted by name (deterministic order).
    pub phases: Vec<PhaseTimings>,
    /// Frame-total median (ms).
    pub frame_median_ms: f32,
    /// Frame-total worst (ms).
    pub frame_max_ms: f32,
    /// Live bytes (from the tracking allocator), MB.
    pub memory_current_mb: f32,
    /// Peak bytes since process start, MB.
    pub memory_peak_mb: f32,
    /// Total allocations since process start.
    pub allocations: usize,
    /// Frames the aggregation covered (0 until the first update).
    pub frames: usize,
}

const BYTES_PER_MB: f32 = 1024.0 * 1024.0;

impl Telemetry {
    /// Recomputes from the last `window` frames of `profiler.history()`.
    ///
    /// A disabled profiler yields an empty `phases` list with `frames: 0`
    /// — the panels then show honest "no data" states rather than zeros
    /// that would look like measured values.
    pub fn update_from(&mut self, profiler: &Profiler, window: usize) {
        let history = profiler.history();
        let tail = history.len().saturating_sub(window);
        *self = aggregate(&history[tail..]);
    }

    /// Status-bar line: `CPU 3.2ms · Mem 12.4 MB` (the reference mock's
    /// CPU/Memory columns; GPU stays an honest omission, ADR-0027).
    pub fn status_summary(&self) -> Option<String> {
        if self.frames == 0 {
            return None;
        }
        Some(format!(
            "CPU {:.1}ms · Mem {:.1} MB",
            self.frame_median_ms, self.memory_current_mb
        ))
    }
}

/// Pure aggregation over a frame slice — the testable core of
/// [`Telemetry::update_from`] (synthetic profiles feed it directly, so no
/// test-only hooks exist on the real capture path).
pub fn aggregate(frames: &[gens_profiler::FrameProfile]) -> Telemetry {
    let mut telemetry = Telemetry {
        frames: frames.len(),
        ..Default::default()
    };

    if frames.is_empty() {
        telemetry.phases.clear();
        telemetry.frame_median_ms = 0.0;
        telemetry.frame_max_ms = 0.0;
    } else {
        let mut frame_totals: Vec<u64> = frames.iter().map(|f| f.total_us).collect();
        frame_totals.sort_unstable();
        telemetry.frame_median_ms = us_to_ms(frame_totals[frame_totals.len() / 2]);
        telemetry.frame_max_ms = us_to_ms(frame_totals[frame_totals.len() - 1]);

        // name -> durations (BTreeMap: phases come out name-sorted)
        let mut per_name: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
        let mut totals: BTreeMap<&str, u64> = BTreeMap::new();
        for frame in frames {
            for span in &frame.spans {
                per_name
                    .entry(span.name.as_str())
                    .or_default()
                    .push(span.dur_us);
                *totals.entry(span.name.as_str()).or_default() += span.dur_us;
            }
        }
        let total_frame_us: u64 = frame_totals.iter().sum();
        telemetry.phases = per_name
            .into_iter()
            .map(|(name, mut durs)| {
                durs.sort_unstable();
                PhaseTimings {
                    name: name.to_string(),
                    median_ms: us_to_ms(durs[durs.len() / 2]),
                    max_ms: us_to_ms(durs[durs.len() - 1]),
                    share: if total_frame_us == 0 {
                        0.0
                    } else {
                        totals[&name] as f32 / total_frame_us as f32
                    },
                }
            })
            .collect();
    }

    let mem = gens_profiler::memory_stats();
    telemetry.memory_current_mb = mem.current_bytes() as f32 / BYTES_PER_MB;
    telemetry.memory_peak_mb = mem.peak_bytes() as f32 / BYTES_PER_MB;
    telemetry.allocations = mem.allocations();
    telemetry
}

fn us_to_ms(us: u64) -> f32 {
    us as f32 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_profiler::{FrameProfile, SpanRecord};

    fn span(name: &str, dur_us: u64) -> SpanRecord {
        SpanRecord {
            name: name.to_string(),
            depth: 0,
            parent: None,
            start_us: 0,
            dur_us,
            orphaned: false,
        }
    }

    fn frame(n: u64, total_us: u64, spans: Vec<SpanRecord>) -> FrameProfile {
        FrameProfile {
            frame: n,
            total_us,
            spans,
            overflowed: false,
        }
    }

    #[test]
    fn aggregates_median_max_and_share() {
        let frames = vec![
            frame(1, 1600, vec![span("scripts", 1000), span("physics", 500)]),
            frame(2, 3800, vec![span("scripts", 3000), span("physics", 700)]),
        ];
        let telemetry = aggregate(&frames);
        assert_eq!(telemetry.frames, 2);
        let by_name = |n: &str| telemetry.phases.iter().find(|p| p.name == n).unwrap();
        // Median convention: sorted[len/2] (upper median).
        assert_eq!(
            by_name("scripts").median_ms,
            3.0,
            "sorted[1000,3000] -> [1] = 3000us"
        );
        assert_eq!(
            by_name("physics").median_ms,
            0.7,
            "sorted[500,700] -> [1] = 700us"
        );
        assert_eq!(by_name("physics").max_ms, 0.7);
        // Frame totals: [1600, 3800] -> median sorted[1] = 3800.
        assert_eq!(telemetry.frame_median_ms, 3.8);
        assert_eq!(telemetry.frame_max_ms, 3.8);
        // Shares: physics total 1200 / frame totals 5400.
        assert!((by_name("physics").share - 1200.0 / 5400.0).abs() < 1e-6);
        // Phases sorted by name.
        let names: Vec<&str> = telemetry.phases.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["physics", "scripts"]);
    }

    #[test]
    fn update_from_limits_to_the_last_n_frames() {
        let mut profiler = Profiler::with_history_cap(64);
        for f in 0..10u64 {
            profiler.begin_frame(f);
            let id = profiler.begin_span("work");
            std::thread::sleep(std::time::Duration::from_micros(50));
            profiler.end_span(id);
            profiler.end_frame();
        }
        let mut telemetry = Telemetry::default();
        telemetry.update_from(&profiler, 3);
        assert_eq!(
            telemetry.frames, 3,
            "window must cover only the last 3 frames"
        );
        assert_eq!(telemetry.phases.len(), 1);
        assert!(telemetry.phases[0].median_ms > 0.0);
    }

    #[test]
    fn disabled_profiler_gives_honest_no_data() {
        let mut telemetry = Telemetry::default();
        let mut profiler = Profiler::disabled();
        profiler.begin_frame(1);
        profiler.end_frame();
        telemetry.update_from(&profiler, 8);
        assert_eq!(telemetry.frames, 0);
        assert!(telemetry.phases.is_empty());
        assert!(
            telemetry.status_summary().is_none(),
            "no data -> no fake CPU line"
        );
    }

    #[test]
    fn status_summary_formats_cpu_and_memory() {
        let telemetry = Telemetry {
            frames: 5,
            frame_median_ms: 3.24,
            memory_current_mb: 12.44,
            ..Default::default()
        };
        assert_eq!(
            telemetry.status_summary().unwrap(),
            "CPU 3.2ms · Mem 12.4 MB"
        );
    }

    #[test]
    fn real_profiler_history_flows_through() {
        // End-to-end through the real capture path.
        let mut profiler = Profiler::enabled();
        profiler.begin_frame(1);
        profiler.time("scripts", |_| {});
        profiler.time("physics", |_| {});
        profiler.end_frame();
        let mut telemetry = Telemetry::default();
        telemetry.update_from(&profiler, 8);
        assert_eq!(telemetry.frames, 1);
        assert_eq!(telemetry.phases.len(), 2);
        assert!(telemetry.status_summary().is_some());
    }
}
