//! Aggregation of frame profiles into a report, with JSON export (P8-2).
//!
//! The report is a pure function of the recorded [`FrameProfile`]s: given
//! the same history it produces the same numbers and the same JSON bytes
//! (spans sorted by name, fixed field order via serde). Timing itself is
//! wall-clock and varies run to run — that is expected and disclosed; what
//! must stay deterministic is the *shape* of the report and everything the
//! simulation produces (ADR-0027).

use crate::spans::FrameProfile;
use serde::Serialize;
use std::collections::BTreeMap;

/// Min / median / p99 / max over a sample set, in microseconds.
///
/// Percentile convention (documented, deterministic): samples are sorted
/// ascending; median = `sorted[len / 2]`, p99 = `sorted[min(len-1, len*99/100)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Quartiles {
    /// Smallest sample.
    pub min_us: u64,
    /// `sorted[len/2]` of the samples.
    pub median_us: u64,
    /// `sorted[min(len-1, len*99/100)]` of the samples.
    pub p99_us: u64,
    /// Largest sample.
    pub max_us: u64,
}

/// Aggregate statistics for one span name across the aggregated frames.
#[derive(Debug, Clone, Serialize)]
pub struct SpanStats {
    /// Span name.
    pub name: String,
    /// Total occurrences across all frames.
    pub count: u64,
    /// Frames containing at least one such span.
    pub present_frames: u64,
    /// Duration quartiles over all occurrences.
    #[serde(flatten)]
    pub us: Quartiles,
    /// Sum of all occurrence durations divided by the sum of all frame
    /// totals (0.0 for an empty report) — the share of wall time this name
    /// consumed, averaged over frames.
    pub share_of_frame: f64,
}

/// Aggregated profile report over a set of frames.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileReport {
    /// Format tag, versioned like every engine format (GENSPROF1).
    pub format: &'static str,
    /// Number of frames aggregated.
    pub frames: u64,
    /// Frame-total quartiles (wall clock from `begin_frame` to `end_frame`).
    #[serde(rename = "frame_us")]
    pub frame_totals: Quartiles,
    /// Per-span-name statistics, sorted by name (deterministic order).
    pub spans: Vec<SpanStats>,
}

/// The report's format tag.
pub const PROFILE_FORMAT: &str = "gens-profile-1";

/// Builds the report from a frame history (newest-last is irrelevant to
/// the result; aggregation is order-independent except for percentile
/// indexing, which is itself sorted first).
pub fn report_from_history(history: &[FrameProfile]) -> ProfileReport {
    if history.is_empty() {
        return ProfileReport {
            format: PROFILE_FORMAT,
            frames: 0,
            frame_totals: Quartiles {
                min_us: 0,
                median_us: 0,
                p99_us: 0,
                max_us: 0,
            },
            spans: Vec::new(),
        };
    }

    let frame_total_samples: Vec<u64> = history.iter().map(|f| f.total_us).collect();
    let frame_totals = quartiles(&frame_total_samples);
    let total_frame_us: u64 = frame_total_samples.iter().sum();

    // name -> (durations, present-frames)
    let mut per_name: BTreeMap<&str, (Vec<u64>, u64)> = BTreeMap::new();
    for frame in history {
        let mut touched: BTreeMap<&str, ()> = BTreeMap::new();
        for span in &frame.spans {
            per_name
                .entry(span.name.as_str())
                .or_default()
                .0
                .push(span.dur_us);
            touched.entry(span.name.as_str()).or_default();
        }
        for name in touched.keys() {
            per_name.entry(name).or_default().1 += 1;
        }
    }

    let spans = per_name
        .into_iter()
        .map(|(name, (mut durs, present))| {
            durs.sort_unstable();
            let total: u64 = durs.iter().sum();
            let count = durs.len() as u64;
            SpanStats {
                name: name.to_string(),
                count,
                present_frames: present,
                us: quartiles_sorted(&durs),
                share_of_frame: if total_frame_us == 0 {
                    0.0
                } else {
                    total as f64 / total_frame_us as f64
                },
            }
        })
        .collect();

    ProfileReport {
        format: PROFILE_FORMAT,
        frames: history.len() as u64,
        frame_totals,
        spans,
    }
}

impl ProfileReport {
    /// Serializes the report to compact JSON bytes (fixed field order via
    /// struct serialization — byte-stable for identical reports).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ProfileReport serializes")
    }
}

fn quartiles(samples: &[u64]) -> Quartiles {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    quartiles_sorted(&sorted)
}

fn quartiles_sorted(sorted: &[u64]) -> Quartiles {
    debug_assert!(!sorted.is_empty(), "quartiles of empty sample set");
    let pick = |idx: usize| sorted[idx.min(sorted.len() - 1)];
    Quartiles {
        min_us: sorted[0],
        median_us: pick(sorted.len() / 2),
        p99_us: pick(sorted.len() * 99 / 100),
        max_us: sorted[sorted.len() - 1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spans::SpanRecord;

    fn span(name: &str, depth: u16, dur_us: u64) -> SpanRecord {
        SpanRecord {
            name: name.to_string(),
            depth,
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
    fn empty_history_gives_empty_report() {
        let report = report_from_history(&[]);
        assert_eq!(report.frames, 0);
        assert!(report.spans.is_empty());
        assert_eq!(report.format, "gens-profile-1");
        let json = report.to_json();
        assert!(json.contains(r#""format":"gens-profile-1""#));
        assert!(json.contains(r#""frames":0"#));
    }

    #[test]
    fn aggregation_sums_counts_and_frames_present() {
        // physics present in all 3 frames; scripts only in frame 3.
        let history = vec![
            frame(1, 1000, vec![span("physics", 1, 400), span("step", 0, 950)]),
            frame(2, 1000, vec![span("physics", 1, 500), span("step", 0, 960)]),
            frame(
                3,
                1000,
                vec![
                    span("physics", 1, 600),
                    span("scripts", 1, 100),
                    span("step", 0, 990),
                ],
            ),
        ];
        let report = report_from_history(&history);
        assert_eq!(report.frames, 3);
        assert_eq!(
            report.frame_totals,
            Quartiles {
                min_us: 1000,
                median_us: 1000,
                p99_us: 1000,
                max_us: 1000
            }
        );

        let by_name = |n: &str| report.spans.iter().find(|s| s.name == n).unwrap();
        let physics = by_name("physics");
        assert_eq!(physics.count, 3);
        assert_eq!(physics.present_frames, 3);
        assert_eq!(physics.us.min_us, 400);
        assert_eq!(physics.us.median_us, 500);
        assert_eq!(physics.us.max_us, 600);
        assert_eq!(physics.us.p99_us, 600, "3 samples: len*99/100 = 2 -> max");

        let scripts = by_name("scripts");
        assert_eq!(scripts.count, 1);
        assert_eq!(scripts.present_frames, 1);
        assert_eq!(scripts.us.median_us, 100);

        // share_of_frame: physics total 1500 / frame totals 3000 = 0.5.
        assert!((physics.share_of_frame - 0.5).abs() < 1e-9);
        assert!((scripts.share_of_frame - 100.0 / 3000.0).abs() < 1e-9);

        // Deterministic order: sorted by name.
        let names: Vec<&str> = report.spans.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["physics", "scripts", "step"]);
    }

    #[test]
    fn percentile_convention_on_100_samples() {
        // Samples 1..=100: median = sorted[50] = 51, p99 = sorted[99] = 100.
        let samples: Vec<u64> = (1..=100).collect();
        let q = quartiles(&samples);
        assert_eq!(q.min_us, 1);
        assert_eq!(q.median_us, 51);
        assert_eq!(q.p99_us, 100);
        assert_eq!(q.max_us, 100);
    }

    #[test]
    fn percentile_convention_on_single_sample() {
        let q = quartiles(&[42]);
        assert_eq!(
            q,
            Quartiles {
                min_us: 42,
                median_us: 42,
                p99_us: 42,
                max_us: 42
            }
        );
    }

    #[test]
    fn json_is_byte_stable_and_complete() {
        let history = vec![
            frame(1, 1000, vec![span("physics", 1, 400), span("step", 0, 950)]),
            frame(2, 1000, vec![span("physics", 1, 500), span("step", 0, 960)]),
        ];
        let json = report_from_history(&history).to_json();
        let json2 = report_from_history(&history).to_json();
        assert_eq!(json, json2, "same history must give byte-identical JSON");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["format"], "gens-profile-1");
        assert_eq!(parsed["frames"], 2);
        assert_eq!(parsed["frame_us"]["median_us"], 1000);
        let spans = parsed["spans"].as_array().unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0]["name"], "physics");
        assert_eq!(spans[0]["count"], 2);
        assert_eq!(spans[0]["present_frames"], 2);
        assert_eq!(spans[0]["min_us"], 400);
        assert_eq!(
            spans[0]["median_us"], 500,
            "2 samples: sorted[len/2] = sorted[1]"
        );
        assert_eq!(spans[0]["max_us"], 500);
        assert!(spans[0]["share_of_frame"].as_f64().unwrap() > 0.4);
    }

    #[test]
    fn report_is_order_independent_over_the_same_multiset() {
        let a = vec![
            frame(1, 100, vec![span("x", 0, 10)]),
            frame(2, 200, vec![span("x", 0, 20)]),
        ];
        let b = vec![
            frame(2, 200, vec![span("x", 0, 20)]),
            frame(1, 100, vec![span("x", 0, 10)]),
        ];
        let ra = report_from_history(&a);
        let rb = report_from_history(&b);
        assert_eq!(ra.to_json(), rb.to_json());
    }

    #[test]
    fn orphan_flags_aggregate_regardless() {
        // Aggregation ignores depth/parent/orphan status by design; it only
        // reads name + duration. (Flags stay available on raw profiles.)
        let mut orphan = span("x", 1, 10);
        orphan.orphaned = true;
        let history = vec![frame(1, 100, vec![orphan])];
        let report = report_from_history(&history);
        assert_eq!(report.spans[0].count, 1);
        assert_eq!(report.spans[0].us.median_us, 10);
    }
}
