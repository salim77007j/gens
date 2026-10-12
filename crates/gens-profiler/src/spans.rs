//! Handle-based nested span capture (Phase 8, P8-1).
//!
//! Contract (ADR-0027): the profiler observes wall-clock time around
//! caller-chosen scopes and never feeds any measured value back into the
//! simulation. Measurement is two `Instant::now()` reads per span plus a
//! `Vec` push — observation only.
//!
//! Robustness rules (a profiler must never take the simulation down):
//! - `begin_frame` while a frame is open auto-completes the previous frame
//!   (open spans are closed and flagged `orphaned`), so a forgotten
//!   `end_frame` cannot poison the next frame.
//! - `end_span(id)` for a non-innermost id closes every span above `id`
//!   (flagged `orphaned`) and then `id` itself, so misuse is visible in the
//!   data instead of panicking.
//! - Per-frame span capture is capped ([`Profiler::MAX_SPANS_PER_FRAME`]);
//!   beyond the cap the frame is flagged `overflowed` and further spans are
//!   ignored — memory stays bounded even on runaway instrumentation.
//! - A disabled profiler ([`Profiler::disabled`]) costs one branch per API
//!   call and records nothing.

use std::time::Instant;

/// Handle to an open span, valid until the frame ends.
///
/// The invalid id ([`SpanId::INVALID`]) is returned by a disabled profiler
/// and is a no-op everywhere it is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanId(u32);

impl SpanId {
    /// The do-nothing id.
    pub const INVALID: SpanId = SpanId(u32::MAX);
}

/// One completed, timed span.
#[derive(Debug, Clone)]
pub struct SpanRecord {
    /// Caller-chosen scope name (`"physics"`, `"scripts"`, ...).
    pub name: String,
    /// Nesting depth (0 = direct child of the frame).
    pub depth: u16,
    /// Parent span, if nested.
    pub parent: Option<SpanId>,
    /// Microseconds after frame start where the span began.
    pub start_us: u64,
    /// Wall-clock duration in microseconds.
    pub dur_us: u64,
    /// True when this span was force-closed by a contract repair
    /// (auto-completed frame or out-of-order `end_span`) rather than by its
    /// own matching `end_span` call.
    pub orphaned: bool,
}

/// Profile of one completed frame.
#[derive(Debug, Clone, Default)]
pub struct FrameProfile {
    /// Caller-assigned frame number (pass through from the game loop).
    pub frame: u64,
    /// Microseconds from `begin_frame` to `end_frame` (wall clock).
    pub total_us: u64,
    /// Completed spans in completion order.
    pub spans: Vec<SpanRecord>,
    /// True when the per-frame span cap was hit (spans were dropped).
    pub overflowed: bool,
}

/// Entry on the open-span stack: id, name, depth, parent, start instant.
#[derive(Debug)]
struct OpenSpan {
    id: SpanId,
    name: String,
    depth: u16,
    parent: Option<SpanId>,
    start: Instant,
}

/// Wall-clock span profiler (observation-only, ADR-0027).
///
/// Typical use around an engine step:
///
/// ```
/// use gens_profiler::Profiler;
/// let mut p = Profiler::enabled();
/// p.begin_frame(7);
/// p.time("step", |p| {
///     p.time("physics", |_| { /* the real step */ });
///     p.time("scripts", |_| { /* the real step */ });
/// });
/// let profile = p.end_frame().expect("frame recorded");
/// assert_eq!(profile.frame, 7);
/// assert_eq!(profile.spans.len(), 3);
/// ```
#[derive(Debug)]
pub struct Profiler {
    enabled: bool,
    frame_start: Option<Instant>,
    current_frame: u64,
    open: Vec<OpenSpan>,
    spans: Vec<SpanRecord>,
    next_id: u32,
    overflowed: bool,
    history: Vec<FrameProfile>,
    history_cap: usize,
}

impl Default for Profiler {
    fn default() -> Self {
        Self::disabled()
    }
}

impl Profiler {
    /// Maximum spans captured per frame; further `begin_span` calls are
    /// ignored (with the `overflowed` flag) so memory stays bounded.
    pub const MAX_SPANS_PER_FRAME: usize = 4096;

    /// Default number of completed frames kept in history.
    pub const DEFAULT_HISTORY_CAP: usize = 600;

    /// A recording profiler.
    pub fn enabled() -> Profiler {
        Profiler::build(true)
    }

    /// A no-op profiler (records nothing, costs one branch per call).
    pub fn disabled() -> Profiler {
        Profiler::build(false)
    }

    /// A recording profiler with a custom completed-frame history cap.
    pub fn with_history_cap(cap: usize) -> Profiler {
        let mut p = Profiler::build(true);
        p.history_cap = cap;
        p
    }

    fn build(enabled: bool) -> Profiler {
        Profiler {
            enabled,
            frame_start: None,
            current_frame: 0,
            open: Vec::new(),
            spans: Vec::new(),
            next_id: 0,
            overflowed: false,
            history: Vec::new(),
            history_cap: Self::DEFAULT_HISTORY_CAP,
        }
    }

    /// Whether this profiler records.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Starts frame `frame`. If a previous frame was still open it is
    /// completed first (open spans flagged `orphaned`) — see the module
    /// contract.
    pub fn begin_frame(&mut self, frame: u64) {
        if !self.enabled {
            return;
        }
        if self.frame_start.is_some() {
            self.end_frame();
        }
        self.frame_start = Some(Instant::now());
        self.current_frame = frame;
        self.spans.clear();
        self.open.clear();
        self.overflowed = false;
        self.next_id = 0;
    }

    /// Opens a named span. Returns [`SpanId::INVALID`] on a disabled
    /// profiler or a capped frame; both are no-ops for `end_span`.
    pub fn begin_span(&mut self, name: &str) -> SpanId {
        if !self.enabled
            || self.frame_start.is_none()
            || self.spans.len() + self.open.len() >= Self::MAX_SPANS_PER_FRAME
        {
            if self.enabled && self.frame_start.is_some() {
                self.overflowed = true;
            }
            return SpanId::INVALID;
        }
        let id = SpanId(self.next_id);
        self.next_id += 1;
        let depth = self.open.len() as u16;
        let parent = self.open.last().map(|o| o.id);
        self.open.push(OpenSpan {
            id,
            name: name.to_string(),
            depth,
            parent,
            start: Instant::now(),
        });
        id
    }

    /// Closes the span `id`. If `id` is not the innermost open span, every
    /// span above it is force-closed and flagged `orphaned` (module
    /// contract). `SpanId::INVALID` and unknown ids are no-ops.
    pub fn end_span(&mut self, id: SpanId) {
        if !self.enabled || id == SpanId::INVALID || self.frame_start.is_none() {
            return;
        }
        let Some(pos) = self.open.iter().rposition(|o| o.id == id) else {
            return; // unknown id (already closed): silent no-op
        };
        let frame_start = self.frame_start.unwrap();
        // Force-close everything above `pos` first, flagged orphaned.
        while self.open.len() > pos + 1 {
            self.close_top(frame_start, true);
        }
        self.close_top(frame_start, false);
    }

    /// Times `f` as a span named `name`, passing `&mut self` to `f` so
    /// scopes nest naturally. Returns `f`'s value. On a disabled profiler
    /// this is a plain call with the closure receiving the (still
    /// disabled) profiler.
    pub fn time<T>(&mut self, name: &str, f: impl FnOnce(&mut Self) -> T) -> T {
        let id = self.begin_span(name);
        let out = f(self);
        self.end_span(id);
        out
    }

    /// Completes the open frame (if any) and returns its
    /// [`FrameProfile`], pushing it onto the capped history.
    pub fn end_frame(&mut self) -> Option<FrameProfile> {
        if !self.enabled {
            return None;
        }
        let frame_start = self.frame_start.take()?;
        while let Some(_open) = self.open.last() {
            self.close_top(frame_start, true);
        }
        let total_us = frame_start.elapsed().as_micros() as u64;
        let profile = FrameProfile {
            frame: self.current_frame,
            total_us,
            spans: std::mem::take(&mut self.spans),
            overflowed: std::mem::take(&mut self.overflowed),
        };
        self.history.push(profile.clone());
        if self.history.len() > self.history_cap {
            self.history.remove(0);
        }
        Some(profile)
    }

    /// Completed frames currently held (newest last), capped.
    pub fn history(&self) -> &[FrameProfile] {
        &self.history
    }

    /// Number of completed frames held.
    pub fn frames_recorded(&self) -> usize {
        self.history.len()
    }

    /// Drops all completed frames.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Takes the entire history (used by the report exporter).
    pub fn take_history(&mut self) -> Vec<FrameProfile> {
        std::mem::take(&mut self.history)
    }

    fn close_top(&mut self, frame_start: Instant, orphaned: bool) {
        if let Some(open) = self.open.pop() {
            // Offset of the span's start within the frame; `open.start` is
            // created after `frame_start`, so this saturates to a plain
            // subtraction (never negative).
            let start_us = open.start.duration_since(frame_start).as_micros() as u64;
            let dur_us = open.start.elapsed().as_micros() as u64;
            self.spans.push(SpanRecord {
                name: open.name,
                depth: open.depth,
                parent: open.parent,
                start_us,
                dur_us,
                orphaned,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    /// ~100us of guaranteed-real work so duration assertions are safe.
    fn busy() {
        let mut acc = 0u64;
        for i in 0..20_000u64 {
            acc = acc.wrapping_add(i.wrapping_mul(6_364_136_223_846_793_005));
        }
        black_box(acc);
    }

    #[test]
    fn nesting_records_depth_parent_and_order() {
        let mut p = Profiler::enabled();
        p.begin_frame(1);
        let outer = p.begin_span("step");
        busy();
        let a = p.begin_span("physics");
        busy();
        p.end_span(a);
        let b = p.begin_span("scripts");
        p.end_span(b);
        p.end_span(outer);
        let frame = p.end_frame().expect("frame");
        assert_eq!(frame.frame, 1);
        assert_eq!(frame.spans.len(), 3);
        assert_eq!(frame.spans[0].name, "physics");
        assert_eq!(frame.spans[0].depth, 1);
        assert_eq!(frame.spans[0].parent, Some(outer));
        assert_eq!(frame.spans[1].name, "scripts");
        assert_eq!(frame.spans[2].name, "step");
        assert_eq!(frame.spans[2].depth, 0);
        assert!(frame.spans[2].parent.is_none());
        // All completed inside the frame: none flagged orphaned.
        assert!(frame.spans.iter().all(|s| !s.orphaned));
        // Inner spans complete before the outer span ends: durations nest.
        let outer = &frame.spans[2];
        let inner_end = frame.spans[0].start_us + frame.spans[0].dur_us;
        assert!(inner_end <= outer.start_us + outer.dur_us + 1); // +1us slack
    }

    #[test]
    fn time_closure_wraps_and_returns() {
        let mut p = Profiler::enabled();
        p.begin_frame(2);
        let value = p.time("step", |p| p.time("physics", |_| 41u32 + 1));
        assert_eq!(value, 42);
        let frame = p.end_frame().expect("frame");
        assert_eq!(frame.spans.len(), 2);
        assert_eq!(frame.spans[1].name, "step");
    }

    #[test]
    fn durations_are_positive_for_real_work() {
        let mut p = Profiler::enabled();
        p.begin_frame(3);
        p.time("work", |_| busy());
        let frame = p.end_frame().expect("frame");
        assert!(
            frame.spans[0].dur_us > 0,
            "20k-iteration loop must take >1us"
        );
        assert!(frame.total_us >= frame.spans[0].dur_us);
    }

    #[test]
    fn disabled_profiler_is_a_no_op() {
        let mut p = Profiler::disabled();
        p.begin_frame(1);
        let id = p.begin_span("x");
        assert_eq!(id, SpanId::INVALID);
        p.time("y", |_| {});
        p.end_span(id);
        assert!(p.end_frame().is_none());
        assert_eq!(p.frames_recorded(), 0);
        assert!(p.history().is_empty());
    }

    #[test]
    fn out_of_order_end_flags_orphans() {
        let mut p = Profiler::enabled();
        p.begin_frame(4);
        let outer = p.begin_span("outer");
        let middle = p.begin_span("middle");
        let inner = p.begin_span("inner");
        // Close the MIDDLE first: inner force-closes orphaned, then middle;
        // outer still closes normally at its own end_span.
        p.end_span(middle);
        p.end_span(inner); // already closed: silent no-op
        p.end_span(outer);
        let frame = p.end_frame().expect("frame");
        let by_name = |n: &str| frame.spans.iter().find(|s| s.name == n).unwrap();
        assert!(by_name("inner").orphaned, "inner must be flagged orphaned");
        assert!(!by_name("middle").orphaned, "middle closed by its own call");
        assert!(!by_name("outer").orphaned);
        assert_eq!(frame.spans.len(), 3, "no duplicate records for the same id");
    }

    #[test]
    fn begin_frame_auto_completes_a_still_open_frame() {
        let mut p = Profiler::enabled();
        p.begin_frame(1);
        p.begin_span("forgotten");
        p.begin_frame(2); // auto-completes frame 1
        assert_eq!(p.frames_recorded(), 1);
        let first = &p.history()[0];
        assert_eq!(first.frame, 1);
        assert_eq!(first.spans.len(), 1);
        assert!(first.spans[0].orphaned, "forgotten span must be flagged");
        // Frame 2 records normally.
        p.time("ok", |_| {});
        p.end_frame();
        assert_eq!(p.frames_recorded(), 2);
        assert_eq!(p.history()[1].frame, 2);
    }

    #[test]
    fn history_cap_is_respected() {
        let mut p = Profiler::with_history_cap(5);
        for f in 0..20u64 {
            p.begin_frame(f);
            p.end_frame();
        }
        assert_eq!(p.frames_recorded(), 5);
        assert_eq!(p.history()[0].frame, 15, "oldest frames evicted");
        assert_eq!(p.history()[4].frame, 19);
    }

    #[test]
    fn span_cap_sets_overflowed_and_bounds_memory() {
        let mut p = Profiler::enabled();
        p.begin_frame(1);
        for _ in 0..(Profiler::MAX_SPANS_PER_FRAME + 100) {
            let id = p.begin_span("s");
            p.end_span(id);
        }
        let frame = p.end_frame().expect("frame");
        assert!(frame.overflowed);
        assert!(frame.spans.len() <= Profiler::MAX_SPANS_PER_FRAME);
    }

    #[test]
    fn span_outside_frame_is_ignored() {
        let mut p = Profiler::enabled();
        let id = p.begin_span("no_frame_yet");
        assert_eq!(id, SpanId::INVALID);
        p.end_span(id);
        assert!(p.end_frame().is_none());
    }

    #[test]
    fn take_history_clears() {
        let mut p = Profiler::enabled();
        p.begin_frame(1);
        p.end_frame();
        let taken = p.take_history();
        assert_eq!(taken.len(), 1);
        assert_eq!(p.frames_recorded(), 0);
    }
}
